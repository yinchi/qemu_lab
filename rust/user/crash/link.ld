ENTRY(_start) /** Entry point defined in userlib's start.s */

SECTIONS
{
	/* Same fixed user window as every other EL0 binary this roadmap
	builds -- see hello/link.ld for the full reasoning (the 48 MiB gap
	separating this from the kernel's own region, the "unmapped as a guard
	region" principle it extends). */
	. = 0x44000000;

	.text : { *(.text.boot) *(.text*) }
	.rodata : { *(.rodata*) }

	.eh_frame_hdr : { *(.eh_frame_hdr) }
	.eh_frame : { *(.eh_frame) }

	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }
}
