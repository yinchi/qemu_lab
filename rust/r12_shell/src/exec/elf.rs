//! A minimal ELF64 loader: maps a simple, statically-linked AArch64 executable's `PT_LOAD` segments
//! into the fixed user window `mmu.rs` reserves, using each segment's own `p_flags` for permissions
//! rather than one blanket permission across the whole window. Everything about the file is
//! validated first by `elfparse.rs` (pure, host-tested); this module only copies and maps what that
//! already approved, so a malformed file is an `Err`, never a kernel panic.

use aarch64_paging::descriptor::El1Attributes;
use aarch64_paging::paging::MemoryRegion;

use super::elfparse::{self, ElfError, PF_W, PF_X};
use crate::platform::base_addresses::{USER_BASE, USER_SIZE};
use crate::platform::globals::IDMAP;
use crate::static_mut_ref;

/// Parses `elf_bytes` and maps its `PT_LOAD` segments into the fixed user window. Returns the
/// ELF's entry point -- a user-window address, ready to become `ELR_EL1` for the `eret` into EL0.
///
/// Reused as-is for every program load, not just the first: the TLB invalidate at the end matters
/// from the very second call onward, since this window is the same fixed range for every program
/// this kernel ever loads, and a stale cached translation or permission from whichever program was
/// there before would otherwise still be visible to the CPU.
///
/// # Errors
/// Anything that isn't a simple, statically-linked AArch64 executable whose segments fit inside
/// the user window (see `ElfError`). Nothing is copied or mapped in that case, except for
/// `MapFailed`, which can only surface after earlier segments were already mapped.
pub fn load(elf_bytes: &[u8]) -> Result<usize, ElfError> {
    let parsed = elfparse::parse(elf_bytes, USER_BASE, USER_BASE + USER_SIZE)?;

    let mut result = Ok(parsed.entry);
    for seg in &parsed.segments {
        // Copy the segment's file bytes, then zero the rest of `memsz` -- the tail is `.bss`-like
        // data the file doesn't store since it's all zero.
        let dst = seg.vaddr as *mut u8;
        let src = &elf_bytes[seg.offset..seg.offset + seg.filesz];
        // SAFETY: `parse` checked that `vaddr..vaddr+memsz` lies inside the user window and the
        // source range inside `elf_bytes`; nothing else is using the window while a program is
        // being loaded.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, seg.filesz);
            core::ptr::write_bytes(dst.add(seg.filesz), 0, seg.memsz - seg.filesz);
        }

        // Per-segment permissions straight from the ELF's own p_flags. Every segment this loader
        // ever sees has PF_R set (nothing in this project's toolchain emits an unreadable
        // segment), so there is no separate read-permission bit to derive here.
        let mut attrs = El1Attributes::ATTRIBUTE_INDEX_1
            | El1Attributes::INNER_SHAREABLE
            | El1Attributes::VALID
            | El1Attributes::ACCESSED
            | El1Attributes::USER
            | El1Attributes::PXN; // kernel must never execute user code
        if seg.flags & PF_W == 0 {
            attrs |= El1Attributes::READ_ONLY;
        }
        if seg.flags & PF_X == 0 {
            attrs |= El1Attributes::UXN;
        }

        let region = MemoryRegion::new(seg.vaddr, seg.vaddr + seg.memsz);
        // SAFETY: sole accessor of IDMAP at any given time -- this project runs single-threaded,
        // and nothing else touches the page table while a program is being loaded.
        let mapped = unsafe { static_mut_ref!(IDMAP).map_range(&region, attrs) };
        if mapped.is_err() {
            result = Err(ElfError::MapFailed);
            break;
        }
    }

    // A broad invalidate (every EL1&0 TLB entry, not just this window's pages) rather than a
    // precise per-page one: this runs once per program load, not on any hot path, so simplicity --
    // guaranteed correct regardless of exactly which pages changed -- outweighs the cost of
    // over-invalidating a few entries that didn't need it. Done on the failure path too, since a
    // failed load may have changed some mappings first.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }

    result
}
