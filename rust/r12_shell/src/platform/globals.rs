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
// nothing outside `irq_handler` does. That has changed with Step 5: the shell's loop and the programs
// it runs now have IRQs enabled (they are masked only inside a syscall), so a handler can fire at
// any time -- which is why the keyboard's handler touches only the device and the token queue
// (`keyboard/queue.rs`), never the console, the display, the line discipline or anything else the
// shell uses.
pub static mut BLK: Option<Blk> = None;
pub static mut GPU: Option<Gpu> = None;
pub static mut CONSOLE: Option<Console> = None;
pub static mut KEYBOARD: Option<Keyboard> = None;

// The page table `mmu::enable` builds and activates. Must be kept alive for the program's entire
// remaining life once activated -- `IdMap`'s `Drop` impl panics if an active mapping is ever
// dropped, since that would free memory the CPU is still using as its page table -- and stays
// reachable afterward since the ELF loader edits the user window's entries (`modify_range`) on each
// program load.
pub static mut IDMAP: Option<IdMap<El1And0>> = None;

// SPI numbers for the two interrupt-driven devices, filled in once each right before that
// device's GIC line is enabled. `irq_handler` reads these to route an acknowledged interrupt.
pub static BLK_SPI: AtomicU32 = AtomicU32::new(0);
pub static KEYBOARD_SPI: AtomicU32 = AtomicU32::new(0);
