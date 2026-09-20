//! Device statics for the IRQ-driven kernel -- reached from `main.rs` via `util.rs`'s
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
//! of `Gpu::framebuffer()` (see `drivers/virtio/gpu.rs`, `console.rs`). `IdMap` is `arch/mmu.rs`'s
//! page table, written once there and read again by `exec/elf.rs` on every program load.

use core::sync::atomic::AtomicU32;

use aarch64_paging::{idmap::IdMap, paging::El1And0};

use crate::console::Console;
use crate::drivers::virtio::{blk::Blk, gpu::Gpu, input::Keyboard};

// Every piece of state `irq_handler` needs to reach, handed over from `kernel_main` exactly
// once each, before that device's SPI is ever enabled at the GIC -- see `kernel_main`'s comments
// at each handoff point for why that ordering rules out a race, the same reasoning Stage 3's
// `STATE` static relies on: at most one `irq_handler` invocation ever runs at a time (single
// core, IRQs masked for its duration), so once `kernel_main` stops touching a given static,
// nothing outside `irq_handler` ever does -- true even once a program is running at EL0, since
// `process::run_program` masks every DAIF bit for its entire time there (see `exec/process.rs`'s doc
// comment): a keyboard IRQ simply can't land mid-program to re-enter `irq_handler` while an
// outer call is still on the stack.
pub static mut BLK: Option<Blk> = None;
pub static mut GPU: Option<Gpu> = None;
pub static mut CONSOLE: Option<Console<'static>> = None;
pub static mut KEYBOARD: Option<Keyboard> = None;

// The page table `mmu::enable` builds and activates. Must be kept alive for the program's entire
// remaining life once activated -- `IdMap`'s `Drop` impl panics if an active mapping is ever
// dropped, since that would free memory the CPU is still using as its page table -- and stays
// reachable afterward since the ELF loader calls `map_range` again to remap the user window's
// entries on each program load.
pub static mut IDMAP: Option<IdMap<El1And0>> = None;

// SPI numbers for the two interrupt-driven devices, filled in once each right before that
// device's GIC line is enabled. `irq_handler` reads these to route an acknowledged interrupt.
pub static BLK_SPI: AtomicU32 = AtomicU32::new(0);
pub static KEYBOARD_SPI: AtomicU32 = AtomicU32::new(0);
