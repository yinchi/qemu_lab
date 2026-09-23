//! Builds the page tables and turns the MMU on: one shared identity-mapped table
//! under `TTBR0_EL1` -- MMIO, the kernel image (split by `link.ld`'s
//! boundary symbols into RX/RO/RW+XN regions, the stack set apart from `.bss` by an unmapped
//! guard), and a fixed EL0-accessible
//! user window (filled in by the ELF loader) -- with `TTBR1_EL1` left unused. Stages 9-11 built the
//! same tables but never configured `TCR_EL1` or set `SCTLR_EL1.M`, so translation stayed off and
//! nothing was enforced (see `ROADMAP.md`'s Stage 9 warning and `Stage12.md`'s Step 3); this is
//! where it is actually switched on, together with a few hardening bits. This project runs at most
//! one user program at a time, with no scheduler, so there is never a
//! second resident address space to distinguish from the first: no
//! per-process page tables, no ASIDs, no `TTBR0_EL1` swapping on a context
//! switch, since none of that machinery has anywhere to be useful here.

use core::sync::atomic::{AtomicBool, Ordering};

use aarch64_cpu::registers::{
    ID_AA64MMFR0_EL1, ID_AA64MMFR1_EL1, MAIR_EL1, Readable, SCTLR_EL1, TCR_EL1, Writeable,
};
use aarch64_paging::{
    descriptor::El1Attributes,
    idmap::IdMap,
    paging::{El1And0, MemoryRegion},
};

use crate::platform::base_addresses::{
    GICC_SIZE, GICD_SIZE, UART0_BASE, UART0_SIZE, VIRTIO_MMIO_BASE, VIRTIO_MMIO_SIZE,
};
use crate::platform::globals::IDMAP;

/////////////////////////////////////////////////////////////////////////////////////////////
// MAIR_EL1 attribute indexes: there are 8 1-byte slots, of which we use 0 and 1.

/// Attribute index for device memory in MAIR_EL1.
const ATTR_DEVICE_INDEX: u64 = 0;
/// Attribute index for normal memory in MAIR_EL1.
const ATTR_NORMAL_INDEX: u64 = 1;

/////////////////////////////////////////////////////////////////////////////////////////////

// Kernel image boundary symbols, defined in link.ld.
unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __stack_guard: u8;
    static __stack_bottom: u8;
    static __kernel_end: u8;
}

/// Whether `addr` lies in the unmapped guard below the kernel stack -- what a data abort's
/// `FAR_EL1` says when the stack has overflowed (see `unexpected_exception`).
pub fn in_stack_guard(addr: usize) -> bool {
    // SAFETY: only the addresses of linker-defined boundary symbols are taken.
    unsafe { (sym_addr(&__stack_guard)..sym_addr(&__stack_bottom)).contains(&addr) }
}

/////////////////////////////////////////////////////////////////////////////////////////////

// SCTLR_EL1 bits that switch translation on: M = stage 1 translation, C = data/unified cache,
// I = instruction cache.
const SCTLR_M: u64 = 1 << 0;
const SCTLR_C: u64 = 1 << 2;
const SCTLR_I: u64 = 1 << 12;

// SCTLR_EL1 hardening bits: SA / SA0 = fault on a misaligned stack pointer used as a base at EL1 /
// EL0; WXN = a page that is writable is never executable, whatever its own execute bit says; SPAN
// = 0 makes every exception into EL1 set PSTATE.PAN (see `user_access`).
const SCTLR_SA: u64 = 1 << 3;
const SCTLR_SA0: u64 = 1 << 4;
const SCTLR_WXN: u64 = 1 << 19;
const SCTLR_SPAN: u64 = 1 << 23;

/// PSTATE.PAN (Privileged Access Never), as the `PAN` system register presents it (bit 22).
const PAN_BIT: u64 = 1 << 22;

/// Whether PAN is on (the CPU has FEAT_PAN and `enable` turned it on). `user_access` only touches
/// the `PAN` register if so, since it is undefined on a CPU without the feature.
static PAN_ENABLED: AtomicBool = AtomicBool::new(false);

/// What `enable` turned on beyond translation itself -- reported at boot.
pub struct Hardening {
    pub pan: bool,
}

/// Root level of the page table.  A level-1 root holds up to 512 1 GiB entries, i.e. 512^3
/// 4 KiB pages.
const ROOT_LEVEL: usize = 1;

/// Address Space Identifier (ASID) for the root page table.
/// Only ever one address space -- no per-process ASIDs needed.
const ASID: usize = 0;

/// Extracts the address of a linker-defined boundary symbol.
///
/// # Safety
/// `s` must be one of this module's `extern "C"` boundary symbols.
unsafe fn sym_addr(s: &u8) -> usize {
    s as *const u8 as usize
}

/// Turns the MMU on. Called once, early in `kernel_main`, after
/// `init_base_addresses` has parsed the DTB (so `gicd`/`gicc` below are
/// known) but before `gic_setup`/any device access -- see `kernel_main`'s
/// ordering.
pub fn enable(gicd: usize, gicc: usize) -> Hardening {
    // Device-nGnRE: non-Gathering, non-Reordering, no Early write
    // acknowledgement -- the standard "MMIO register access" encoding.
    const MAIR_DEVICE_NGNRE: u64 = 0b0000_0100;

    // Normal memory, Inner/Outer Write-Back, Read/Write-Allocate, Non-transient.
    const MAIR_NORMAL: u64 = 0xff;

    MAIR_EL1.set(
        (MAIR_DEVICE_NGNRE << (8 * ATTR_DEVICE_INDEX)) | (MAIR_NORMAL << (8 * ATTR_NORMAL_INDEX)),
    );

    let mut idmap = IdMap::with_asid(ASID, ROOT_LEVEL, El1And0);

    let kernel_base = El1Attributes::ATTRIBUTE_INDEX_1
        | El1Attributes::INNER_SHAREABLE
        | El1Attributes::VALID
        | El1Attributes::ACCESSED;

    let kernel_attr_rx = El1Attributes::READ_ONLY | El1Attributes::UXN;
    let kernel_attr_ro = kernel_attr_rx | El1Attributes::PXN;
    let kernel_attr_rw = El1Attributes::UXN | El1Attributes::PXN;

    let device = El1Attributes::ATTRIBUTE_INDEX_0
        | El1Attributes::VALID
        | El1Attributes::ACCESSED
        | El1Attributes::UXN
        | El1Attributes::PXN;
    let kernel_rx = kernel_base | kernel_attr_rx;
    let kernel_ro = kernel_base | kernel_attr_ro;
    let kernel_rw = kernel_base | kernel_attr_rw;

    // 1. MMIO: GIC distributor+CPU interface, UART, the virtio-mmio window.
    idmap
        .map_range(&MemoryRegion::new(gicd, gicd + GICD_SIZE), device)
        .unwrap();
    idmap
        .map_range(&MemoryRegion::new(gicc, gicc + GICC_SIZE), device)
        .unwrap();
    idmap
        .map_range(
            &MemoryRegion::new(UART0_BASE, UART0_BASE + UART0_SIZE),
            device,
        )
        .unwrap();
    idmap
        .map_range(
            &MemoryRegion::new(VIRTIO_MMIO_BASE, VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE),
            device,
        )
        .unwrap();

    // 2. Kernel image, split by link.ld's boundary symbols.
    unsafe {
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__text_start), sym_addr(&__text_end)),
                kernel_rx,
            )
            .unwrap();
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__rodata_start), sym_addr(&__rodata_end)),
                kernel_ro,
            )
            .unwrap();
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__data_start), sym_addr(&__data_end)),
                kernel_rw,
            )
            .unwrap();
        // The stack, with the guard below it (`__stack_guard..__stack_bottom`) deliberately left
        // unmapped: an overflow is a translation fault, not silent corruption of `.bss`.
        idmap
            .map_range(
                &MemoryRegion::new(sym_addr(&__stack_bottom), sym_addr(&__kernel_end)),
                kernel_rw,
            )
            .unwrap();
    }

    // Mapping the fixed user window is left for the ELF loader (`elf.rs`) to handle.

    // TCR_EL1: the translation regime the table above was built for. Nothing else in the project
    // sets it up, and the page table does nothing until the MMU is on, so it all has to be said here.
    //
    // - T0SZ = 25: a 39-bit address space, 512 GiB, which a level-1 root of 512 1 GiB entries
    //   covers exactly (`ROOT_LEVEL`).
    // - TG0 = 4 KiB granule; IRGN0/ORGN0 = write-back, read/write-allocate caching for the table
    //   walks themselves; SH0 = inner shareable.
    // - EPD1 = 1: no TTBR1_EL1 walks, as high memory addresses are not used. (An address outside
    //   the 39 bits translation-faults, which is what a wild user pointer should do.)
    // - IPS: the physical address size the CPU supports (ID_AA64MMFR0_EL1.PARange).
    const T0SZ: u64 = 25;
    const IRGN0_WB_WA: u64 = 0b01 << 8;
    const ORGN0_WB_WA: u64 = 0b01 << 10;
    const SH0_INNER: u64 = 0b11 << 12;
    const TG0_4K: u64 = 0b00 << 14;
    const EPD1: u64 = 1 << 23;
    let parange = ID_AA64MMFR0_EL1.get() & 0xf;
    TCR_EL1.set(T0SZ | IRGN0_WB_WA | ORGN0_WB_WA | SH0_INNER | TG0_4K | EPD1 | (parange << 32));

    // SAFETY: every mapping above covers exactly what this kernel accesses (MMIO, its own image);
    // the user window and the rest of RAM are deliberately left unmapped here instead of mapped and
    // unused. `activate` points TTBR0_EL1 at the table.
    unsafe {
        idmap.activate();
    }

    // Turn translation on (SCTLR_EL1.M), together with the data and instruction caches (C, I).
    // Before this the CPU ignored the page table entirely: every address was its own physical address
    // and nothing was protected from anything. Stale TLB entries can't exist yet, but the barrier
    // sequence is the one the architecture requires around the write.
    // SAFETY: the table maps the code executing this instruction and everything it touches next.
    unsafe {
        core::arch::asm!("dsb ish", "isb");
    }
    // Beyond turning translation on, the checks that catch our own mistakes early: WXN (nothing is
    // ever both writable and executable -- no mapping here is), and stack-pointer alignment
    // checks (the AArch64 ABI keeps SP 16-byte aligned; every stack this project sets up does).
    // Deliberately *not* enabled: SCTLR_EL1.A, which faults on every unaligned access, including
    // the plain unaligned loads Rust emits for `read_unaligned` (`elfparse.rs`).
    let mut sctlr =
        SCTLR_EL1.get() | SCTLR_M | SCTLR_C | SCTLR_I | SCTLR_WXN | SCTLR_SA | SCTLR_SA0;

    // PAN, if the CPU has it (ID_AA64MMFR1_EL1.PAN, bits 23:20): with it set the kernel faults on
    // any access to a page EL0 may access, so a stray kernel dereference of a user pointer is a
    // fault instead of a silent success. Kernel code that means to touch user memory wraps that in
    // `user_access()`. SPAN = 0 makes every exception entry set it again, so the syscall and fault
    // paths start protected without having to remember to.
    let pan = (ID_AA64MMFR1_EL1.get() >> 20) & 0xf != 0;
    if pan {
        sctlr &= !SCTLR_SPAN;
    }
    SCTLR_EL1.set(sctlr);
    // SAFETY: as above.
    unsafe {
        core::arch::asm!("isb");
    }
    if pan {
        set_pan(true);
        PAN_ENABLED.store(true, Ordering::Relaxed);
    }

    // SAFETY: sole write to IDMAP, happening once, here, before anything else could reach it.
    // Keeping it alive is not optional: `IdMap`'s `Drop` impl panics if an active mapping is ever
    // dropped, since that would free memory the CPU is still using as its page table.
    unsafe {
        IDMAP = Some(idmap);
    }
    Hardening { pan }
}

/// Writes PSTATE.PAN through the `PAN` system register (S3_0_C4_C2_3; the assembler's `msr pan, #imm`
/// needs a newer baseline than this target's).
fn set_pan(on: bool) {
    let value = if on { PAN_BIT } else { 0 };
    // SAFETY: only called when the CPU has FEAT_PAN (see `enable`, `user_access`).
    unsafe { core::arch::asm!("msr S3_0_C4_C2_3, {v}", "isb", v = in(reg) value) };
}

/// Lets the kernel touch user memory for as long as the returned guard lives, by clearing PAN.
/// Held only around the few places that do -- the loader, `argv` setup, and the syscalls that take
/// user pointers. Does nothing if the CPU has no PAN.
#[must_use = "PAN is only cleared while the guard is alive"]
pub fn user_access() -> UserAccess {
    if PAN_ENABLED.load(Ordering::Relaxed) {
        set_pan(false);
    }
    UserAccess
}

pub struct UserAccess;

impl Drop for UserAccess {
    fn drop(&mut self) {
        if PAN_ENABLED.load(Ordering::Relaxed) {
            set_pan(true);
        }
    }
}
