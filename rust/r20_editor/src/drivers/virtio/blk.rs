//! Finds the VirtIO block devices among the `virtio,mmio` slots `platform/base_addresses.rs`
//! discovered -- every one, since Stage 19 (earlier stages had exactly one).
//!
//! Unlike Stage 6's copy of this file, this stage drives the block device through a real GIC
//! interrupt rather than polling: `VirtIOBlk` exposes an async submit/complete pair
//! (`read_blocks_nb`/`write_blocks_nb` + `complete_read_blocks`/`complete_write_blocks`) plus
//! `ack_interrupt()`, so a genuine IRQ-driven transfer is possible here -- unlike `virtio-gpu`
//! (see `gpu.rs`'s doc comment), which only exposes synchronous, internally-polling calls.

use virtio_drivers::Error;
use virtio_drivers::device::blk::{BlkReq, BlkResp, VirtIOBlk};
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::mmio::MmioTransport;

use super::{hal::VirtioHalImpl, mmio_transports};
use crate::platform::globals::BLK;

/// The most block devices the kernel drives: `BLK` and `BLK_SPI` have this many entries. A device past the
/// last is left alone (`Blk::find_all` reports how many it skipped).
pub const MAX_BLK: usize = 4;

/// Block device `dev` (an index into `BLK`).
///
/// # Safety
/// `dev` must be below the count of devices `kernel_main` populated `BLK` with, and the caller must be the only
/// user of `BLK` in the way `platform/globals.rs` describes (single core; `irq_handler` touches only
/// `ack_interrupt`).
pub unsafe fn get(dev: usize) -> &'static mut Blk {
    #[allow(clippy::deref_addrof, reason = "`&raw mut` is what the 2024 edition's `static_mut_refs` lint asks for")]
    unsafe {
        (*(&raw mut BLK))[dev].as_mut().unwrap()
    }
}

/// Wrapper around a VirtIO block device, providing IRQ-driven read and write operations.
pub struct Blk {
    inner: VirtIOBlk<VirtioHalImpl, MmioTransport<'static>>,
}

impl Blk {
    /// Every discovered `virtio,mmio` slot that turns out to be a block device, in slot order, each with its
    /// SPI number -- up to `MAX_BLK`, and the number left over (their SPIs are never enabled, so they stay silent).
    /// A device whose driver fails to initialize is skipped like an empty slot.
    pub fn find_all(
        mmio_slots: impl Iterator<Item = (usize, u32)>,
    ) -> ([Option<(Self, u32)>; MAX_BLK], usize) {
        let mut found = [const { None }; MAX_BLK];
        let mut count = 0;
        let mut skipped = 0;
        for (transport, irq) in mmio_transports(mmio_slots, DeviceType::Block) {
            if count == MAX_BLK {
                skipped += 1;
                continue;
            }
            let Ok(mut inner) = VirtIOBlk::<VirtioHalImpl, _>::new(transport) else {
                continue;
            };
            // Enable interrupts for this block device (no-op since the HAL-provided DMA memory
            // is already zeroed which enables interrupts by default).
            inner.enable_interrupts();
            found[count] = Some((Self { inner }, irq));
            count += 1;
        }
        (found, skipped)
    }

    // Read/write flow: submit request, wait for interrupt, irq_handler calls `ack_interrupt` to
    // signal completion, GIC then returns execution to the waiting function (read/write).

    /// Reads one or more sectors into `buf`, IRQ-driven: submits the request, then sleeps
    /// (`wfe`) until this device's SPI -- acknowledged by `ack_interrupt` below, called from
    /// `main.rs`'s `irq_handler` -- signals completion.
    ///
    /// Safe to call despite wrapping two `unsafe` primitives: `req`/`resp` are local to this
    /// function and untouched by anything else while the request is in flight (the wait loop
    /// below only ever reads `peek_used()`), and the exact same `req`/`buf`/`resp` are passed to
    /// both `read_blocks_nb` and `complete_read_blocks` -- the two things their own SAFETY
    /// comments require, discharged here once instead of pushed out to every caller.
    pub fn read_blocks_irq(&mut self, block_id: usize, buf: &mut [u8]) -> Result<(), Error> {
        let mut req = BlkReq::default();
        let mut resp = BlkResp::default();

        // Submit the read request (non-blocking) and obtain a token to track its completion.
        // SAFETY: see this method's doc comment.
        let token = unsafe {
            self.inner
                .read_blocks_nb(block_id, &mut req, buf, &mut resp)?
        };

        // Wait for the interrupt to signal completion.
        while self.inner.peek_used() != Some(token) {
            unsafe { core::arch::asm!("wfe") };
        }

        // SAFETY: see this method's doc comment.
        unsafe { self.inner.complete_read_blocks(token, &req, buf, &mut resp) }
    }

    /// Writes `buf` to one or more sectors, IRQ-driven -- same shape and safety reasoning as
    /// `read_blocks_irq`.
    pub fn write_blocks_irq(&mut self, block_id: usize, buf: &[u8]) -> Result<(), Error> {
        let mut req = BlkReq::default();
        let mut resp = BlkResp::default();

        // Submit the write request (non-blocking) and obtain a token to track its completion.
        // SAFETY: see read_blocks_irq's doc comment -- identical reasoning.
        let token = unsafe {
            self.inner
                .write_blocks_nb(block_id, &mut req, buf, &mut resp)?
        };

        // Wait for the interrupt to signal completion.
        while self.inner.peek_used() != Some(token) {
            unsafe { core::arch::asm!("wfe") };
        }

        // SAFETY: see read_blocks_irq's doc comment.
        unsafe {
            self.inner
                .complete_write_blocks(token, &req, buf, &mut resp)
        }
    }

    /// Acknowledge interrupt from `irq_handler` once identified as for this device's SPI,
    /// which returns execution to the waiting function (read_blocks_irq/write_blocks_irq).
    pub fn ack_interrupt(&mut self) {
        self.inner.ack_interrupt();
    }

    /// Device capacity in bytes (`VirtIOBlk::capacity` is in 512-byte sectors) -- `fs/blkio.rs`'s
    /// byte-addressable view of the device needs this as the endpoint `SeekFrom::End` measures
    /// from, and to know when a read has run past the device's actual size.
    pub fn capacity_bytes(&self) -> u64 {
        self.inner.capacity() * 512
    }
}
