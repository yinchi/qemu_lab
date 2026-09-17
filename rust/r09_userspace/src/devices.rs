//! Device statics for the IRQ-driven kernel -- reached from `main.rs`/`fat_io.rs`/`mmu.rs`/
//! `elf.rs`/`console.rs` via `utils.rs`'s `static_mut_ref!`/`static_ref!`/`static_mut_opt!`
//! macros.
//!
//! `Blk`/`Gpu` are thin wrappers around the underlying VirtIO driver structs (`VirtIOBlk`,
//! `VirtIOGpu`). `Console` isn't a VirtIO wrapper at all -- it's the software text-rendering layer
//! built on top of `Gpu::framebuffer()` (see `gpu.rs`, `console.rs`). `IdMap` is `mmu.rs`'s page
//! table, written once there and read again by `elf.rs` on every program load.

use core::sync::atomic::AtomicU32;

use aarch64_paging::{idmap::IdMap, paging::El1And0};

use crate::{blk::Blk, console::Console, gpu::Gpu};

// The page table `mmu::enable` builds and activates. Must be kept alive for the program's entire
// remaining life once activated -- `IdMap`'s `Drop` impl panics if an active mapping is ever
// dropped, since that would free memory the CPU is still using as its page table -- and stays
// reachable afterward since the ELF loader calls `map_range` again to remap the user window's
// entries on each program load.
pub static mut IDMAP: Option<IdMap<El1And0>> = None;

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
