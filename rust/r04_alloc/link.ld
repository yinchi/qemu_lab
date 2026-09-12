ENTRY(_start) /** Entry point defined in boot.s */

SECTIONS
{
	/* QEMU's "virt" machine loads -kernel images at this address */
	. = 0x40000000;

    /* .text.boot comes from boot.s and must be aligned to
    0x40000000, thus it comes first in our section ordering.

    All other sections come from main.rs or are left empty. */

	.text : { *(.text.boot) *(.text*) }
	.rodata : { *(.rodata*) }

	/* Explicitly placed (rather than left as orphans) after observing them
	land *before* .text.boot in a release build despite landing after it in
	debug -- an orphan section's position isn't guaranteed consistent across
	build profiles, only across identical ones. force-unwind-tables=no
	already removes these entirely in release; this is a backstop for debug
	builds (or any future profile) where that isn't guaranteed to hold. */
	.eh_frame_hdr : { *(.eh_frame_hdr) }
	.eh_frame : { *(.eh_frame) }

	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }

    /* Define a stack: the stack starts at its highest reserved address
    and grows downwards */

	. = ALIGN(16);  /* Align the stack pointer to a 16-byte boundary */
	. = . + 0x4000; /* Allocate 16 KB for the stack */
	stack_top = .;  /* Define the top of the stack */
}
