//! Enables the MMU, turned on for the first time anywhere in this project
//! (see `ROADMAP.md`'s Stage 9): one shared identity-mapped page table
//! under `TTBR0_EL1` -- MMIO, the kernel image (split by `link.ld`'s
//! boundary symbols into RX/RO/RW+XN regions), and a fixed EL0-accessible
//! user window -- with `TTBR1_EL1` left unused. This project runs at most
//! one user program at a time, with no scheduler, so there is never a
//! second resident address space to distinguish from the first: no
//! per-process page tables, no ASIDs, no `TTBR0_EL1` swapping on a context
//! switch, since none of that machinery has anywhere to be useful here.

use aarch64_cpu::registers::{MAIR_EL1, Readable, TCR_EL1, Writeable};
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
    static __kernel_end: u8;
}

/////////////////////////////////////////////////////////////////////////////////////////////

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
pub fn enable(gicd: usize, gicc: usize) {
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
                &MemoryRegion::new(sym_addr(&__data_start), sym_addr(&__kernel_end)),
                kernel_rw,
            )
            .unwrap();
    }

    // Mapping the fixed user window is left for the ELF loader (`elf.rs`) to handle.

    // SAFETY: every mapping above covers exactly what this kernel accesses
    // (MMIO, its own image); the user window and the rest of RAM are
    // deliberately left unmapped here instead of mapped and unused.
    unsafe {
        idmap.activate();
    }

    // Explicitly disable TTBR1_EL1 walks (TCR_EL1.EPD1, bit 23) as we do not use
    // high memory addresses through TTBR1_EL1.
    const EPD1: u64 = 1 << 23;
    TCR_EL1.set(TCR_EL1.get() | EPD1);

    // SAFETY: sole write to IDMAP, happening once, here, before anything else could reach it.
    // Keeping it alive is not optional: `IdMap`'s `Drop` impl panics if an active mapping is ever
    // dropped, since that would free memory the CPU is still using as its page table.
    unsafe {
        IDMAP = Some(idmap);
    }
}
