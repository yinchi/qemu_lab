//! A minimal ELF64 loader: puts a simple, statically-linked AArch64 executable's `PT_LOAD` segments
//! and a fresh stack into the user window (`platform/base_addresses.rs`), mapping every page with its
//! own permissions -- code read+execute, read-only data read-only, everything else read+write and
//! never executable -- and leaving the guard below the stack unmapped. Everything about the file is
//! validated first by `elfparse.rs` (pure, host-tested); this module only copies and maps what that
//! already approved, so a malformed file is an `Err`, never a kernel panic.
//!
//! The window is a *ceiling* (32 MiB), not an allocation: a program is given exactly the pages its
//! segments (`p_vaddr` to `p_vaddr + p_memsz`, so its `.bss` counts) and its stack need, however many
//! that is up to the ceiling. Every load starts by unmapping exactly the pages the previous program
//! was given -- recorded in `MAPPED` as they were mapped, so even a load that failed half way is undone
//! by the next -- which is what stops a small program inheriting a large one's leftover memory. The pages
//! a segment lands on are then mapped *writable* just long enough to fill (the kernel's own writes obey
//! the page permissions too, now that the MMU is on), and locked down to the segment's real permissions
//! afterwards. What ends up mapped, and how, is recorded in `usermem` so a syscall can check a user
//! pointer before touching it.

use alloc::vec::Vec;

use aarch64_paging::descriptor::{El1Attributes, PhysicalAddress};
use aarch64_paging::paging::MemoryRegion;

use super::elfparse::{self, ElfError, PAGE_SIZE, PF_W, PF_X, Segment};
use super::usermem::UserMemory;
use crate::arch::mmu;
use crate::platform::base_addresses::{
    USER_BASE, USER_IMAGE_END, USER_STACK_BOTTOM, USER_STACK_TOP,
};
use crate::platform::globals::IDMAP;
use crate::static_mut_ref;

/// What is mapped in the user window right now. Written only by `load`; read by syscalls.
/// SAFETY (every access): single core, and neither `load` nor a syscall runs while the other does.
static mut USER_MEMORY: UserMemory = UserMemory::new();

/// The page ranges (`start..end`) the last load mapped, in the order it mapped them. The next load unmaps
/// them before it maps anything, so nothing one program was given outlives it.
/// SAFETY (every access): as `USER_MEMORY`.
static mut MAPPED: Vec<(usize, usize)> = Vec::new();

/// The user memory the running program has mapped (see `usermem.rs`).
#[allow(clippy::deref_addrof)]
pub fn user_memory() -> &'static UserMemory {
    // SAFETY: see USER_MEMORY.
    unsafe { &*(&raw const USER_MEMORY) }
}

/// Attributes every user page shares: normal cacheable memory, EL0-accessible, accessed already,
/// and never executable by the kernel.
const USER_PAGE: El1Attributes = El1Attributes::ATTRIBUTE_INDEX_1
    .union(El1Attributes::INNER_SHAREABLE)
    .union(El1Attributes::VALID)
    .union(El1Attributes::ACCESSED)
    .union(El1Attributes::USER)
    .union(El1Attributes::PXN);

/// A segment's final permissions, straight from the ELF's own `p_flags`. Every segment this loader
/// ever sees is readable (nothing in this project's toolchain emits an unreadable one).
fn final_attributes(flags: u32) -> El1Attributes {
    let mut attrs = USER_PAGE;
    if flags & PF_W == 0 {
        attrs |= El1Attributes::READ_ONLY;
    }
    if flags & PF_X == 0 {
        attrs |= El1Attributes::UXN;
    }
    attrs
}

/// Read+write, never executable: what a page is while being filled, and what the stack is for good.
fn data_attributes() -> El1Attributes {
    USER_PAGE | El1Attributes::UXN
}

/// The page-aligned extent of `seg`'s memory.
fn pages_of(seg: &Segment) -> (usize, usize) {
    (
        seg.vaddr & !(PAGE_SIZE - 1),
        (seg.vaddr + seg.memsz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1),
    )
}

/// Sets every page of `start..end` to `attrs`, or unmaps them if `None`. A page with no mapping yet
/// is simply mapped; one already mapped may only have its permissions changed (the page table
/// refuses anything else, which is what keeps the CPU's view consistent -- see `aarch64-paging`'s
/// break-before-make rules). Identity mapped, so a page's physical address is its virtual one.
fn set_pages(start: usize, end: usize, attrs: Option<El1Attributes>) -> Result<(), ElfError> {
    let region = MemoryRegion::new(start, end);
    // SAFETY: sole accessor of IDMAP at any given time -- this project runs single-threaded, and
    // nothing else touches the page table while a program is being loaded.
    unsafe { static_mut_ref!(IDMAP) }
        .modify_range(&region, &|chunk, descriptor| {
            let flags = match attrs {
                // A level-3 entry is a page descriptor, which is marked as such.
                Some(a) if descriptor.level() == 3 => a | El1Attributes::TABLE_OR_PAGE,
                Some(a) => a,
                None => El1Attributes::empty(),
            };
            descriptor.set(PhysicalAddress(chunk.start().0), flags)
        })
        .map_err(|_| ElfError::MapFailed)
}

/// Makes the instructions just written to `start..end` visible to instruction fetch: the CPU's data
/// and instruction caches are separate, and the kernel wrote the code through the former.
fn sync_instruction_cache(start: usize, end: usize) {
    // CTR_EL0.DminLine (bits 19:16): log2 of the smallest data cache line, in 4-byte words.
    let ctr: usize;
    // SAFETY: reads an ID register.
    unsafe { core::arch::asm!("mrs {c}, ctr_el0", c = out(reg) ctr) };
    let line = 4usize << ((ctr >> 16) & 0xf);
    let mut addr = start & !(line - 1);
    while addr < end {
        // SAFETY: cleans one cache line of memory this function's caller just wrote.
        unsafe { core::arch::asm!("dc cvau, {a}", a = in(reg) addr) };
        addr += line;
    }
    // SAFETY: barrier and instruction-cache invalidate, no memory effects of their own.
    unsafe {
        core::arch::asm!("dsb ish", "ic ialluis", "dsb ish", "isb");
    }
}

/// Parses `elf_bytes` and installs it -- segments and stack -- in the user window. Returns the ELF's
/// entry point, ready to become `ELR_EL1` for the `eret` into EL0.
///
/// Reused as-is for every program load: it begins by unmapping whatever the previous program had,
/// so nothing carries over.
///
/// # Errors
/// Returns an `Err(ElfError)` if the ELF is invalid or cannot be mapped. Nothing is copied or
/// mapped if the file itself is bad; `ElfError::MapFailed` can only surface after the window was
/// already partly rebuilt, which is fine: the next load starts by unmapping it all again.
pub fn load(elf_bytes: &[u8]) -> Result<usize, ElfError> {
    let parsed = elfparse::parse(elf_bytes, USER_BASE, USER_IMAGE_END)?;

    // SAFETY: see USER_MEMORY.
    #[allow(clippy::deref_addrof)]
    let memory = unsafe { &mut *(&raw mut USER_MEMORY) };
    memory.clear();

    // The kernel writes user pages below (zeroing, copying), which PAN would otherwise forbid.
    let _user = mmu::user_access();

    // SAFETY: see MAPPED.
    #[allow(clippy::deref_addrof)]
    let mapped = unsafe { &mut *(&raw mut MAPPED) };

    // 1. Nothing of the previous program stays mapped. (A range leaves the list only once it is
    //    unmapped, so a failure here leaves the rest recorded for the next try.)
    while let Some(&(start, end)) = mapped.last() {
        set_pages(start, end, None)?;
        mapped.pop();
    }

    // 2. Each segment's pages, writable while they are filled: zeroed whole (a page is never left
    //    holding the previous program's bytes), then the file's bytes copied in.
    for seg in &parsed.segments {
        let (start, end) = pages_of(seg);
        set_pages(start, end, Some(data_attributes()))?;
        mapped.push((start, end));
        let src = &elf_bytes[seg.offset..seg.offset + seg.filesz];
        // SAFETY: `parse` checked the segment lies inside the image part of the window and the
        // source range inside `elf_bytes`; the pages were just mapped writable and nothing else
        // uses the window while a program is being loaded.
        unsafe {
            core::ptr::write_bytes(start as *mut u8, 0, end - start);
            core::ptr::copy_nonoverlapping(src.as_ptr(), seg.vaddr as *mut u8, seg.filesz);
        }
    }

    // 3. The stack, zeroed the same way.
    set_pages(USER_STACK_BOTTOM, USER_STACK_TOP, Some(data_attributes()))?;
    mapped.push((USER_STACK_BOTTOM, USER_STACK_TOP));
    // SAFETY: just mapped writable, nothing else uses it.
    unsafe {
        core::ptr::write_bytes(
            USER_STACK_BOTTOM as *mut u8,
            0,
            USER_STACK_TOP - USER_STACK_BOTTOM,
        );
    }
    memory.add(USER_STACK_BOTTOM, USER_STACK_TOP, true);

    // 4. Lock each segment down to its own permissions, and make code visible to instruction fetch.
    for seg in &parsed.segments {
        let (start, end) = pages_of(seg);
        set_pages(start, end, Some(final_attributes(seg.flags)))?;
        memory.add(start, end, seg.flags & PF_W != 0);
        if seg.flags & PF_X != 0 {
            sync_instruction_cache(start, end);
        }
    }

    // The page table code invalidates the TLB entry of every page it changes; this final broad
    // invalidate is belt and braces, and cheap next to a program load.
    // SAFETY: barriers and a TLB invalidate, no memory effects.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }

    Ok(parsed.entry)
}
