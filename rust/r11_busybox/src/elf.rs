//! A minimal ELF64 loader: parses a simple, statically-linked AArch64
//! executable and maps its `PT_LOAD` segments into the fixed user window
//! `mmu.rs` reserves, using each segment's own `p_flags` for permissions
//! rather than one blanket permission across the whole window.

use aarch64_paging::descriptor::El1Attributes;
use aarch64_paging::paging::MemoryRegion;

use crate::base_addresses::{USER_BASE, USER_SIZE};
use crate::devices::IDMAP;
use crate::static_mut_ref;

/// Size, in bytes, of `e_ident`.
const EI_NIDENT: usize = 16;

/// `e_ident[EI_CLASS]` value meaning "64-bit objects" (as opposed to `1` for 32-bit).
const ELFCLASS64: u8 = 2;

/// `e_ident[EI_DATA]` value meaning little-endian (as opposed to `2`, big-endian).
const ELFDATA2LSB: u8 = 1;

/// `e_type` value meaning a statically-linked executable.
const ET_EXEC: u16 = 2;

/// `e_machine` value identifying the target instruction set as AArch64.
const EM_AARCH64: u16 = 183;

/// Program header `p_type` value meaning "load this segment into memory at runtime".
const PT_LOAD: u32 = 1;

/// `p_flags` bit meaning the segment should be mapped executable.
const PF_X: u32 = 1;

/// `p_flags` bit meaning the segment should be mapped writable.
const PF_W: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    /// ELF identification segment
    e_ident: [u8; EI_NIDENT],
    /// Object file type, must be ET_EXEC for `load` to succeed.
    e_type: u16,
    /// Target instruction set architecture, must be EM_AARCH64 for `load` to succeed.
    e_machine: u16,
    /// Object file version.
    e_version: u32,
    /// Entry point virtual address.
    e_entry: u64,
    /// Program header table file offset.
    e_phoff: u64,
    /// Section header table file offset -- unused: this loader only ever walks `PT_LOAD` program
    /// headers, never section headers (no symbol resolution or debugging happens here).
    e_shoff: u64,
    /// Processor-specific flags -- unused; always `0` for AArch64's standard ABI.
    e_flags: u32,
    /// Size of this header, in bytes -- unused; this struct's own `size_of` already gives it.
    e_ehsize: u16,
    /// Size of one program header table entry, in bytes.
    e_phentsize: u16,
    /// Number of entries in the program header table.
    e_phnum: u16,
    /// Size of one section header table entry, in bytes -- unused, see `e_shoff`.
    e_shentsize: u16,
    /// Number of entries in the section header table -- unused, see `e_shoff`.
    e_shnum: u16,
    /// Section header table index of the section name string table -- unused, see `e_shoff`.
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64ProgramHeader {
    /// Segment type -- `load` only maps segments where this is `PT_LOAD`, skipping everything
    /// else.
    p_type: u32,
    /// Segment permission flags (`PF_X`/`PF_W`; `PF_R` is assumed set on every segment this
    /// loader ever sees, per the module doc comment).
    p_flags: u32,
    /// Offset of the segment's data within the ELF file.
    p_offset: u64,
    /// Virtual address at which the segment should be mapped.
    p_vaddr: u64,
    /// Physical address -- unused; this platform has no physical/virtual distinction beyond what
    /// `mmu.rs`'s identity mapping already provides.
    p_paddr: u64,
    /// Size of the segment's data within the file.
    p_filesz: u64,
    /// Size the segment should occupy in memory once loaded (>= `p_filesz`; the remainder is
    /// zero-filled -- `.bss`-like data the file doesn't need to store since it's all zero).
    p_memsz: u64,
    /// Required alignment for `p_offset`/`p_vaddr` -- unused; this loader doesn't enforce or
    /// depend on any particular alignment beyond what the segments already satisfy.
    p_align: u64,
}

/// Reads a `T` out of `data` at byte offset `off`, unaligned -- ELF fields
/// aren't guaranteed to land on a Rust-friendly alignment boundary.
///
/// # Safety
/// `off + size_of::<T>()` must be within `data`'s bounds (checked by the
/// caller via the `assert!` below, not by this function itself).
unsafe fn read_at<T: Copy>(data: &[u8], off: usize) -> T {
    assert!(
        off + core::mem::size_of::<T>() <= data.len(),
        "ELF field out of bounds"
    );
    unsafe { (data.as_ptr().add(off) as *const T).read_unaligned() }
}

/// Parses `elf_bytes` and maps its `PT_LOAD` segments into the fixed user
/// window. Returns the ELF's entry point -- a user-window address, ready
/// to become `ELR_EL1` for the `eret` into EL0.
///
/// Reused as-is for every program load, not just the first: the TLB
/// invalidate at the end matters from the very second call onward, since
/// this window is the same fixed range for every program this kernel ever
/// loads, and a stale cached translation or permission from whichever
/// program was there before would otherwise still be visible to the CPU.
///
/// # Panics
/// On anything that doesn't look like a simple, statically-linked AArch64
/// executable, or a segment that doesn't fit inside the fixed user window --
/// every binary this loader ever sees comes from this project's own build,
/// so a malformed one is a build-time mistake to fix, not a runtime
/// condition to recover from gracefully.
pub fn load(elf_bytes: &[u8]) -> usize {
    let header: Elf64Header = unsafe { read_at(elf_bytes, 0) };

    // Basic sanity checks on the ELF header before proceeding.
    assert_eq!(&header.e_ident[0..4], b"\x7fELF", "not an ELF file");
    assert_eq!(header.e_ident[4], ELFCLASS64, "not a 64-bit ELF");
    assert_eq!(header.e_ident[5], ELFDATA2LSB, "not little-endian");
    assert_eq!(header.e_type, ET_EXEC, "not a statically-linked executable");
    assert_eq!(header.e_machine, EM_AARCH64, "not an AArch64 binary");

    for i in 0..header.e_phnum as usize {
        let off = header.e_phoff as usize + i * header.e_phentsize as usize;
        let phdr: Elf64ProgramHeader = unsafe { read_at(elf_bytes, off) };
        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr as usize;
        let filesz = phdr.p_filesz as usize;
        let memsz = phdr.p_memsz as usize;
        assert!(
            vaddr >= USER_BASE && vaddr + memsz <= USER_BASE + USER_SIZE,
            "PT_LOAD segment doesn't fit inside the user window"
        );

        // Copy the segment's file bytes, then zero the rest of p_memsz --
        // the ELF spec guarantees memsz >= filesz, and the tail is
        // .bss-like data the file doesn't store since it's all zero.
        let dst = vaddr as *mut u8;
        let src = &elf_bytes[phdr.p_offset as usize..phdr.p_offset as usize + filesz];
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, filesz);
            core::ptr::write_bytes(dst.add(filesz), 0, memsz - filesz);
        }

        // Per-segment permissions straight from the ELF's own p_flags.
        // Every segment this loader ever sees has PF_R set (nothing in
        // this project's toolchain emits an unreadable segment), so there
        // is no separate read-permission bit to derive here.
        let mut attrs = El1Attributes::ATTRIBUTE_INDEX_1
            | El1Attributes::INNER_SHAREABLE
            | El1Attributes::VALID
            | El1Attributes::ACCESSED
            | El1Attributes::USER
            | El1Attributes::PXN; // kernel must never execute user code
        if phdr.p_flags & PF_W == 0 {
            attrs |= El1Attributes::READ_ONLY;
        }
        if phdr.p_flags & PF_X == 0 {
            attrs |= El1Attributes::UXN;
        }

        let region = MemoryRegion::new(vaddr, vaddr + memsz);
        // SAFETY: sole accessor of IDMAP at any given time -- this project
        // runs single-threaded, and nothing else touches the page table
        // while a program is being loaded.
        unsafe {
            static_mut_ref!(IDMAP)
                .map_range(&region, attrs)
                .expect("failed to map PT_LOAD segment");
        }
    }

    // A broad invalidate (every EL1&0 TLB entry, not just this window's
    // pages) rather than a precise per-page one: this runs once per
    // program load, not on any hot path, so simplicity -- guaranteed
    // correct regardless of exactly which pages changed -- outweighs the
    // cost of over-invalidating a few entries that didn't need it.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }

    header.e_entry as usize
}
