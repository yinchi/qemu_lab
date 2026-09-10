ENTRY(_start) /** Entry point defined in start.S */

SECTIONS
{
	/* QEMU's "virt" machine loads -kernel images at this address */
	. = 0x40000000;

    /* .text.boot comes from start.S and must be aligned to
    0x40000000, thus it comes first in our section ordering.
    
    All other sections come from main.c or are left empty. */

	.text : { *(.text.boot) *(.text*) }
	.rodata : { *(.rodata*) }
	.data : { *(.data*) }
	.bss : { *(.bss*) *(COMMON) }

    /* Define a stack: the stack starts at its highest reserved address
    and grows downwards */

	. = ALIGN(16);  /* Align the stack pointer to a 16-byte boundary */
	. = . + 0x4000; /* Allocate 16 KB for the stack */
	stack_top = .;  /* Define the top of the stack */
}
