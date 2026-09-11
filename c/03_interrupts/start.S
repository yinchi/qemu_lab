.section ".text.boot"
.global _start

_start:
	/* Set sp to refer to the processor's stack pointer for EL1 (SP_EL1). */
	/* Does not set it, that is `mov sp, x0` below. */
	msr	SPSel, #1
    /* Instruction Synchronization Barrier -- flush any previous instructions */
	isb

    /* Load the address of the stack top into x0 and set sp to it */
	ldr	x0, =stack_top
	mov	sp, x0

    /* VBAR_EL1 is the register that holds the base address of the exception vector table for EL1.
    Load the address of the vectors table into x0 and write it to VBAR_EL1 */
	ldr	x0, =vectors
	msr	vbar_el1, x0
	isb

    /* Branch-with-link to main */
	bl	main

/* Infinite loop upon return from main */
hang:
	wfe
	b	hang
