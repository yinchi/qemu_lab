//! The virtio-mmio devices this kernel uses, sharing one HAL (`hal`, the DMA pool) and one way of
//! being found (the `virtio,mmio` slots the device tree lists): the block device, the GPU and the
//! input device (the keyboard).

pub mod blk;
pub mod gpu;
pub mod hal;
pub mod input;
