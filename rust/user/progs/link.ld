ENTRY(_start) /** Entry point defined in userlib's start.s */

SECTIONS
{
	/* The kernel maps a fixed 2 MiB EL0-accessible window starting here,
	separated from its own image (from 0x40000000; 6 MiB in the early stages, about 23 MiB in r12)
	by a large unmapped gap -- the "leave it unmapped as a guard region" reasoning: an off-by-one
	in the kernel's own page-table setup lands in genuinely unmapped memory and faults
	immediately, rather than landing ambiguously on an adjacent mapped
	region. Every EL0 binary this roadmap builds links at this same
	address. The kernel sets the initial SP_EL0 to the top of this window
	independently of this binary's own layout, so no stack space needs to
	be reserved here. (From r12 the window is laid out as image, unmapped guard, then a 1 MiB
	stack -- see the kernel's platform/base_addresses.rs; the image must stay under 960 KiB. From r15 the window is a
	32 MiB ceiling and only the pages the image needs are mapped, so an image may be up to about 31 MiB.) */
	. = 0x44000000;

	/* Each of the three loadable segments (code, read-only data, writable data) is mapped with its own
	permissions, and a page can have only one set, so every segment starts on a page boundary. The
	r12+ kernel refuses an executable whose segments share a page. */
	.text : { *(.text.boot) *(.text*) }
	. = ALIGN(4096);
	.rodata : { *(.rodata*) }

	.eh_frame_hdr : { *(.eh_frame_hdr) }
	.eh_frame : { *(.eh_frame) }

	. = ALIGN(4096);
	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }
}
