//! Device statics for the IRQ-driven kernel -- reached from `main.rs`/`fat_io.rs` via `utils.rs`'s
//! `static_mut_ref!`/`static_ref!` macros.
//!
//! `Blk`/`Gpu` are thin wrappers around the underlying VirtIO driver structs (`VirtIOBlk`,
//! `VirtIOGpu`). `Console` isn't a VirtIO wrapper at all -- it's the software text-rendering layer
//! built on top of `Gpu::framebuffer()` (see `gpu.rs`, `console.rs`).

use core::sync::atomic::AtomicU32;

use crate::{blk::Blk, console::Console, gpu::Gpu};

// BLK stays populated (and its SPI enabled) for this program's entire life, not just at boot --
// `fat_io.rs`'s BlkIo reaches through it for every filesystem read/write, not only an initial
// one-shot load the way Stage 6/7's font read used it. Same single-`irq_handler`-at-a-time
// reasoning as those stages' statics otherwise: once `kernel_main` stops touching a given static,
// nothing outside `irq_handler` (BLK's `ack_interrupt`) ever does.
pub static mut BLK: Option<Blk> = None;
pub static mut GPU: Option<Gpu> = None;
pub static mut CONSOLE: Option<Console<'static>> = None;

// SPI number for the block device, filled in once, right before its GIC line is enabled.
// `irq_handler` reads this to route an acknowledged interrupt.
pub static BLK_SPI: AtomicU32 = AtomicU32::new(0);
