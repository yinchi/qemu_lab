//! Finds the VirtIO GPU device among the `virtio,mmio` slots `base_addresses.rs` discovered,
//! negotiates a fixed 640x480 resolution -- matching classic VGA mode 0x11's dimensions, an
//! 80x30 grid at this stage's 8x16 glyph size -- and exposes it as a `console::Framebuffer`.
//! See console.rs's doc comment on why this resolution is chosen once and never renegotiated.
//!
//! Same probing approach as blk.rs: every slot looks identical in the device tree, so each one
//! is tried in turn via `MmioTransport::new`, keeping the first one whose `device_type()` is
//! `GPU`.

use core::ptr::NonNull;

use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::{DeviceType, Transport};

use crate::console::Framebuffer;
use crate::virtio_hal::VirtioHalImpl;

const VIRTIO_MMIO_SIZE: usize = 0x200;

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;

pub struct Gpu {
    inner: VirtIOGpu<VirtioHalImpl, MmioTransport<'static>>,
}

impl Gpu {
    pub fn find(mmio_bases: &[usize]) -> Option<Self> {
        for &base in mmio_bases {
            let header = NonNull::new(base as *mut VirtIOHeader)?;
            // SAFETY: `base` came from a `virtio,mmio` node's `reg` property (see blk.rs's
            // identical reasoning).
            let transport = match unsafe { MmioTransport::new(header, VIRTIO_MMIO_SIZE) } {
                Ok(t) => t,
                Err(_) => continue,
            };
            if transport.device_type() != DeviceType::GPU {
                continue;
            }
            if let Ok(inner) = VirtIOGpu::new(transport) {
                return Some(Self { inner });
            }
        }
        None
    }

    /// Negotiates this module's fixed resolution and returns a `Framebuffer` pointing directly
    /// at the device's DMA-backed pixel buffer -- kept alive for as long as `self` is, since the
    /// backing `Dma` allocation lives inside `self.inner`.
    pub fn framebuffer(&mut self) -> Framebuffer {
        let buf = self
            .inner
            .change_resolution(WIDTH, HEIGHT)
            .expect("failed to set up the virtio-gpu framebuffer");
        Framebuffer {
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
