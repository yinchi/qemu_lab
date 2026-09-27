//! The console's pixel surface: a raw BGRX8888 framebuffer it draws glyphs into.

use crate::drivers::virtio::gpu::FramebufferInfo;

/// A raw BGRX8888 pixel surface. (8 bits per channel, blue-green-red-padding = 32 bits / pixel).
/// Since the machine is little-endian, a color value 0xAARRGGBB (e.g. as a parameter to
/// `put_pixel`) will be stored in memory as BB GG RR AA.
pub struct Framebuffer {
    /// Pointer to the start of the pixel data.
    pub ptr: *mut u8,
    /// Width of the framebuffer in pixels.
    pub width: usize,
    /// Height of the framebuffer in pixels.
    pub height: usize,
    /// Bytes per row; may exceed `width * 4` if the source pads rows.
    pub stride: usize,
}

impl From<FramebufferInfo> for Framebuffer {
    /// Wraps the GPU driver's pixel buffer -- the console owns drawing into it, the driver only
    /// owns the device.
    fn from(info: FramebufferInfo) -> Self {
        Self {
            ptr: info.ptr,
            width: info.width,
            height: info.height,
            stride: info.stride,
        }
    }
}

impl Framebuffer {
    /// Returns the byte offset of the pixel at (`x`, `y`) within the framebuffer's memory.
    fn pixel_offset(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * 4
    }

    /// Sets the pixel at (`x`, `y`) to the given BGRX8888 color.
    pub fn put_pixel(&self, x: usize, y: usize, bgrx: u32) {
        debug_assert!(x < self.width && y < self.height);
        unsafe {
            (self.ptr.add(self.pixel_offset(x, y)) as *mut u32).write_volatile(bgrx);
        }
    }

    /// Gets the BGRX8888 color of the pixel at (`x`, `y`).
    pub fn get_pixel(&self, x: usize, y: usize) -> u32 {
        debug_assert!(x < self.width && y < self.height);
        unsafe { (self.ptr.add(self.pixel_offset(x, y)) as *const u32).read_volatile() }
    }
}
