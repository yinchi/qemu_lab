.section ".text.boot"
.global _start

_start:
    # Load the address of the top of the stack into x0
	ldr	x0, =stack_top
    # Copy the value in x0 to the stack pointer (sp)
	mov	sp, x0
    # Branch (with link) to the kernel_main function
	bl	kernel_main
    # kernel_main returns here
hang:
    # Wait for event
	wfe
    # Branch to hang (infinite loop)
	b	hang
