//! Finds the VirtIO block device among the `virtio,mmio` slots `base_addresses.rs` discovered.
//!
//! Unlike Stage 6's copy of this file, this stage drives the block device through a real GIC
//! interrupt rather than polling: `VirtIOBlk` exposes an async submit/complete pair
//! (`read_blocks_nb`/`write_blocks_nb` + `complete_read_blocks`/`complete_write_blocks`) plus
//! `ack_interrupt()`, so a genuine IRQ-driven transfer is possible here -- unlike `virtio-gpu`
//! (see `gpu.rs`'s doc comment), which only exposes synchronous, internally-polling calls.

use core::ptr::NonNull;

use virtio_drivers::Error;
use virtio_drivers::device::blk::{BlkReq, BlkResp, VirtIOBlk};
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

use crate::virtio_hal::VirtioHalImpl;

/// One `virtio,mmio` slot's region size, per the device tree (`reg = <... 0x200>` on every
/// slot).
const VIRTIO_MMIO_SIZE: usize = 0x200;

/// Wrapper around a VirtIO block device, providing IRQ-driven read and write operations.
pub struct Blk {
    inner: VirtIOBlk<VirtioHalImpl, MmioTransport<'static>>,
}

impl Blk {
    /// Tries every discovered `virtio,mmio` slot in turn and returns a `Blk` (and its SPI
    /// number) for the first one that turns out to be a block device.
    pub fn find(mmio_slots: impl Iterator<Item = (usize, u32)>) -> Option<(Self, u32)> {
        for (base, irq) in mmio_slots {
            let Some(header) = NonNull::new(base as *mut VirtIOHeader) else {
                continue;
            };

            // Attempt to create an MMI/O transport for this slot. Fails naturally if the header
            // isn't valid.
            //
            // SAFETY: `base` came from a `virtio,mmio` node's `reg` property, so it points to a
            // valid VirtIO MMIO region of at least VIRTIO_MMIO_SIZE bytes, for the program's
            // lifetime ('static).
            let transport = match unsafe { MmioTransport::new(header, VIRTIO_MMIO_SIZE) } {
                Ok(t) => t,
                Err(_) => continue, // empty slot, or not a valid VirtIO device at all
            };

            // MMI/O transport successfully created; check if it's a block device.
            if transport.device_type() != DeviceType::Block {
                continue;
            }

            // Block device successfully identified; attempt to create a VirtIOBlk instance.
            // If successful, return it; otherwise, continue searching.
            if let Ok(mut inner) = VirtIOBlk::<VirtioHalImpl, _>::new(transport) {
                // Enable interrupts for this block device (no-op since the HAL-provided DMA memory
                // is already zeroed which enables interrupts by default).
                inner.enable_interrupts();
                return Some((Self { inner }, irq));
            }
        }

        // No block device found among the discovered `virtio,mmio` slots.
        None
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
    /// `read_blocks_irq`. Unused by this stage's demo (only a read is needed to fetch the font)
    /// but kept for symmetry with the underlying driver's own read/write pair.
    #[allow(dead_code)]
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
}
