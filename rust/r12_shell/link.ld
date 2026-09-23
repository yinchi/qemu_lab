ENTRY(_start) /** Entry point defined in boot.s */

SECTIONS
{
	/* QEMU's "virt" machine loads -kernel images at this address */
	. = 0x40000000;

    /* .text.boot comes from boot.s and must be aligned to
    0x40000000, thus it comes first in our section ordering.

    All other sections come from main.rs or are left empty.

    Boundary symbols (__text_start etc.) mark each region for mmu.rs's MMU
    setup to map with distinct permissions -- RX for .text, RO for .rodata
    (and the eh_frame data, also read-only), RW+XN for everything from
    .data/.bss and (beyond an unmapped guard) the stack. */

	__text_start = .;
	.text : { *(.text.boot) *(.text*) }
	/* Each region below is mapped with its own permissions (`arch/mmu.rs`), and a page has only one
	set: a boundary inside a page would give it whichever mapping came last -- .rodata's, which is
	not executable, over the tail of .text, which holds the exception vectors. So every boundary
	is page-aligned. */
	. = ALIGN(4096);
	__text_end = .;

	__rodata_start = .;
	.rodata : { *(.rodata*) }

	/* Explicitly placed (rather than left as orphans) after observing them
	land *before* .text.boot in a release build despite landing after it in
	debug -- an orphan section's position isn't guaranteed consistent across
	build profiles, only across identical ones. force-unwind-tables=no
	already removes these entirely in release; this is a backstop for debug
	builds (or any future profile) where that isn't guaranteed to hold. */
	.eh_frame_hdr : { *(.eh_frame_hdr) }
	.eh_frame : { *(.eh_frame) }
	. = ALIGN(4096);
	__rodata_end = .;

	__data_start = .;
	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }
	. = ALIGN(4096);
	__data_end = .;

    /* The stack starts at its highest reserved address and grows downwards. Below it is a guard:
    address space left unmapped by mmu.rs (which maps __data_start..__data_end and
    __stack_bottom..__kernel_end, and nothing in between), so a stack overflow faults instead of
    silently running into the end of .bss. Larger than any frame the kernel has, so a single large
    frame can't step over it -- the same reasoning as the user stack's guard
    (platform/base_addresses.rs). Page-aligned at both ends, since a mapping is per page. */

	__stack_guard = .;
	. = . + 0x10000;  /* 64 KiB guard, unmapped */
	__stack_bottom = .;
	. = . + 0x100000; /* Allocate 1 MB for the stack */
	stack_top = .;  /* Define the top of the stack */
	__kernel_end = .;
}
