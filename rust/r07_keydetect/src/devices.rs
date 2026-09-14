//! Device statics for the IRQ-driven kernel -- reached from `main.rs` via `utils.rs`'s
//! `static_mut_ref!`/`static_ref!` macros.
//!
//! Syntax:
//!
//! - UPPERCASE: static device handles and SPI numbers (e.g., `BLK`, `GPU`, `BLK_SPI`).
//! - Titlecase: the types of the devices (e.g., `Blk`, `Gpu`, `Console`, `Keyboard`).
//!
//! `Blk`/`Gpu`/`Keyboard` are thin wrappers around the underlying VirtIO driver structs
//! (`VirtIOBlk`, `VirtIOGpu`, `VirtIOInput`). No `VirtIOKeyboard`; the input device is generic.
//!
//! `Console` isn't a VirtIO wrapper at all -- it's the software text-rendering layer built on top
//! of `Gpu::framebuffer()` (see `gpu.rs`, `console.rs`).

use core::sync::atomic::AtomicU32;

use crate::{blk::Blk, console::Console, gpu::Gpu, keyboard::Keyboard};

// Every piece of state `irq_handler` needs to reach, handed over from `kernel_main` exactly
// once each, before that device's SPI is ever enabled at the GIC -- see `kernel_main`'s comments
// at each handoff point for why that ordering rules out a race, the same reasoning Stage 3's
// `STATE` static relies on: at most one `irq_handler` invocation ever runs at a time (single
// core, IRQs masked for its duration), so once `kernel_main` stops touching a given static,
// nothing outside `irq_handler` ever does.
pub static mut BLK: Option<Blk> = None;
pub static mut GPU: Option<Gpu> = None;
pub static mut CONSOLE: Option<Console<'static>> = None;
pub static mut KEYBOARD: Option<Keyboard> = None;

// SPI numbers for the two interrupt-driven devices, filled in once each right before that
// device's GIC line is enabled. `irq_handler` reads these to route an acknowledged interrupt.
pub static BLK_SPI: AtomicU32 = AtomicU32::new(0);
pub static KEYBOARD_SPI: AtomicU32 = AtomicU32::new(0);
