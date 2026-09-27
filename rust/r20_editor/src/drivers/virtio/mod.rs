//! The virtio-mmio devices this kernel uses, sharing one HAL (`hal`, the DMA pool) and one way of
//! being found (the `virtio,mmio` slots the device tree lists): the block device, the GPU and the
//! input device (the keyboard). Several block devices may be present; there is one of each of the others.

pub mod blk;
pub mod gpu;
pub mod hal;
pub mod input;

use core::ptr::NonNull;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

/// The size of the VirtIO MMIO region for any virtio device.
pub const VIRTIO_SLOT_SIZE: usize = 0x200;

/// Every discovered `virtio,mmio` slot that holds a device of the requested `device_type`, as a transport and the
/// slot's IRQ number, in the order the slots were discovered (device-tree order). Slots that are empty or not a
/// valid VirtIO device are skipped silently.
pub fn mmio_transports(
    mmio_slots: impl Iterator<Item = (usize, u32)>,
    device_type: DeviceType,
) -> impl Iterator<Item = (MmioTransport<'static>, u32)> {
    mmio_slots.filter_map(move |(base, irq)| {
        let header = NonNull::new(base as *mut VirtIOHeader)?;

        // Attempt to create an MMI/O transport for this slot. Fails naturally if the header
        // isn't valid.
        //
        // SAFETY: `base` came from a `virtio,mmio` node's `reg` property, so it points to a
        // valid VirtIO MMIO region of at least VIRTIO_SLOT_SIZE bytes, for the program's
        // lifetime ('static).
        let transport = unsafe { MmioTransport::new(header, VIRTIO_SLOT_SIZE) }.ok()?; // empty slot, or not VirtIO
        (transport.device_type() == device_type).then_some((transport, irq))
    })
}

/// The first slot holding a device of the requested `device_type`, with its IRQ number. The GPU and the keyboard
/// are found this way: this machine has one of each, so the first match is the device; a caller whose driver then
/// fails to initialize on it gets `None`, not a search for a second one. (The block devices are all wanted:
/// `mmio_transports`.)
pub fn find_mmio_transport(
    mmio_slots: impl Iterator<Item = (usize, u32)>,
    device_type: DeviceType,
) -> Option<(MmioTransport<'static>, u32)> {
    mmio_transports(mmio_slots, device_type).next()
}
