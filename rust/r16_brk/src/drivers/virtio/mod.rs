//! The virtio-mmio devices this kernel uses, sharing one HAL (`hal`, the DMA pool) and one way of
//! being found (the `virtio,mmio` slots the device tree lists): the block device, the GPU and the
//! input device (the keyboard).

pub mod blk;
pub mod gpu;
pub mod hal;
pub mod input;

use core::ptr::NonNull;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// The size of the VirtIO MMIO region for any virtio device.
pub const VIRTIO_SLOT_SIZE: usize = 0x200;

/// Tries every discovered `virtio,mmio` slot in turn and returns the first one that successfully
/// matches the requested `device_type`, with its IRQ number. This machine has one device of each
/// type, so the first match is the device; a caller whose driver then fails to initialize on it
/// gets `None`, not a search for a second one.
pub fn find_mmio_transport(
    mmio_slots: impl Iterator<Item = (usize, u32)>,
    device_type: DeviceType,
) -> Option<(MmioTransport<'static>, u32)> {
    for (base, irq) in mmio_slots {
        let Some(header) = NonNull::new(base as *mut VirtIOHeader) else {
            continue;
        };

        // Attempt to create an MMI/O transport for this slot. Fails naturally if the header
        // isn't valid.
        //
        // SAFETY: `base` came from a `virtio,mmio` node's `reg` property, so it points to a
        // valid VirtIO MMIO region of at least VIRTIO_SLOT_SIZE bytes, for the program's
        // lifetime ('static).
        let transport = match unsafe { MmioTransport::new(header, VIRTIO_SLOT_SIZE) } {
            Ok(t) => t,
            Err(_) => continue, // empty slot, or not a valid VirtIO device at all
        };

        if transport.device_type() == device_type {
            return Some((transport, irq));
        }
    }

    // No device of the requested type found among the discovered `virtio,mmio` slots.
    None
}
