ENTRY(_start) /** Entry point defined in userlib's start.s */

SECTIONS
{
	/* r09_userspace's MMU setup maps a fixed 2 MiB EL0-accessible window
	starting here, separated from the kernel's own 16 MiB region
	(0x40000000-0x41000000) by a deliberate 48 MiB unmapped gap -- the same
	"leave it unmapped as a guard region" reasoning already used for the
	rest of RAM, applied here too: an off-by-one in the kernel's own
	page-table setup lands in genuinely unmapped memory and faults
	immediately, rather than landing ambiguously on an adjacent mapped
	region. Every EL0 binary this roadmap builds links at this same
	address. The kernel sets the initial SP_EL0 to the top of this window
	independently of this binary's own layout, so no stack space needs to
	be reserved here. */
	. = 0x44000000;

	.text : { *(.text.boot) *(.text*) }
	.rodata : { *(.rodata*) }

	.eh_frame_hdr : { *(.eh_frame_hdr) }
	.eh_frame : { *(.eh_frame) }

	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }
}
