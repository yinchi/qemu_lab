//! Finds the VirtIO GPU device among the `virtio,mmio` slots `platform/base_addresses.rs`
//! discovered, negotiates a fixed 640x480 resolution -- matching classic VGA mode 0x11's
//! dimensions, an 80x30 grid at this stage's 8x16 glyph size -- and exposes its pixel buffer as a
//! `FramebufferInfo` (the console turns that into its own `Framebuffer`; the driver knows nothing
//! about text). See console.rs's doc comment on why this resolution is chosen once and never
//! renegotiated.
//!
//! Same probing approach as blk.rs: every slot looks identical in the device tree, so each one
//! is tried in turn via `MmioTransport::new`, keeping the first one whose `device_type()` is
//! `GPU`.
//!
//! Unlike the block and keyboard devices, *not* interrupt-based: only blocking functions are
//! exposed in the VirtIOGpu driver, so there's no interrupts to wait for.

use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::mmio::MmioTransport;

use super::{find_mmio_transport, hal::VirtioHalImpl};

/// The device's pixel buffer: a BGRX8888 surface (see `console::framebuffer`), `height` rows of
/// `stride` bytes.
pub struct FramebufferInfo {
    /// Pointer to the start of the pixel data.
    pub ptr: *mut u8,
    /// Width of the framebuffer in pixels.
    pub width: usize,
    /// Height of the framebuffer in pixels.
    pub height: usize,
    /// Bytes per row; may exceed `width * 4` if the source pads rows.
    pub stride: usize,
}

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;

pub struct Gpu {
    inner: VirtIOGpu<VirtioHalImpl, MmioTransport<'static>>,
}

impl Gpu {
    /// Finds the GPU among the discovered `virtio,mmio` slots. Polled, so its IRQ number is unused.
    pub fn find(mmio_slots: impl Iterator<Item = (usize, u32)>) -> Option<Self> {
        let (transport, _irq) = find_mmio_transport(mmio_slots, DeviceType::GPU)?;
        let inner = VirtIOGpu::new(transport).ok()?;
        Some(Self { inner })
    }

    /// Negotiates this module's fixed resolution and returns the device's DMA-backed pixel buffer
    /// -- kept alive for as long as `self` is, since the backing `Dma` allocation lives inside
    /// `self.inner`.
    pub fn framebuffer(&mut self) -> FramebufferInfo {
        let buf = self
            .inner
            .change_resolution(WIDTH, HEIGHT)
            .expect("failed to set up the virtio-gpu framebuffer");
        FramebufferInfo {
            ptr: buf.as_mut_ptr(),
            width: WIDTH as usize,
            height: HEIGHT as usize,
            stride: WIDTH as usize * 4,
        }
    }

    /// Pushes whatever's currently in the framebuffer to the actual display.
    pub fn flush(&mut self) {
        self.inner
            .flush()
            .expect("failed to flush the virtio-gpu framebuffer");
    }
}
