//! Finds the VirtIO block device among the `virtio,mmio` (memory-mapped I/O) slots
//! `base_addresses.rs` discovered, and reads sectors from it.

use core::ptr::NonNull;

use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

use crate::virtio_hal::VirtioHalImpl;

/// One `virtio,mmio` slot's region size, per the device tree (`reg = <... 0x200>` on every
/// slot).
const VIRTIO_MMIO_SIZE: usize = 0x200;

/// Tries every discovered `virtio,mmio` slot in turn and returns a `VirtIOBlk` for the first one
/// that turns out to be a block device.
pub fn find_block_device(
    mmio_bases: &[usize],
) -> Option<VirtIOBlk<VirtioHalImpl, MmioTransport<'static>>> {
    for &base in mmio_bases {
        // Create a non-null pointer to the VirtIO header at this MMIO base address.
        let header = NonNull::new(base as *mut VirtIOHeader)?;

        // Attempt to create an MMI/O transport for this slot.  Fails naturally if the header
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
        if let Ok(blk) = VirtIOBlk::<VirtioHalImpl, _>::new(transport) {
            return Some(blk);
        }
    }

    // No block device found among the discovered `virtio,mmio` slots.
    None
}
