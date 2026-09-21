/* Defines assembly routines for saving and restoring the kernel context when transitioning
between EL1 and EL0.

enter_el0: Entry point for transitioning from EL1 to EL0 and jumping to the EL0 program.
resume_kernel: Entry point for resuming execution in the kernel after returning from EL0.

The trick that makes this simple: enter_el0 upholds the ordinary AAPCS64 calling convention
exactly -- callee-saved registers (x19-x30) and SP survive across it, caller-saved registers don't
need to -- so from Rust's own point of view, calling it is indistinguishable from calling any other
function that happens to take a very long time (during which a whole EL0 program runs) before
returning. No flag-checking or special-casing is needed at the call site. */

.section .bss
.align 3 /* 2^3 = 8-byte alignment */

/* Reserve 14 registers worth of space within .bss: sp, resume_addr, x19-x30 (12 regs) */
.global KERNEL_CTX
KERNEL_CTX:
	.skip 14 * 8

.section .text

/* Saves this project's own callee-saved register set (x19-x30) and SP into KERNEL_CTX, alongside
the address to resume at (the local label below), then erets into EL0 -- SPSR_EL1/ELR_EL1/SP_EL0
are already set by the caller (process.rs's run_program). Called with a plain `bl`, like any other
function.

Once exit()/a caught fault (syscall.rs) calls resume_kernel, execution returns here -- not via
`eret`'s own continuation (that genuinely never returns), but via resume_kernel restoring every
register saved below and branching directly to the local label, at which point this function does
an entirely ordinary `ret`, using the x30 that was live when enter_el0 was first called. x0 is
left as resume_kernel's argument -- the program's exit status -- so to the caller it is simply
enter_el0's return value (a longjmp value, in setjmp/longjmp terms). */
.global enter_el0
enter_el0:

	/* Save the current SP to x9, and the address to resume at to x10 */
	mov	x9, sp
	adr	x10, 1f /* Next label `1` in the forward direction (save to x10) */

	/* Load the address of KERNEL_CTX into x11 */
	ldr	x11, =KERNEL_CTX

	/* Store SP (x9), resume address (x10), and callee-saved registers starting at KERNEL_CTX */
	stp	x9, x10, [x11, #0]
	stp	x19, x20, [x11, #16]
	stp	x21, x22, [x11, #32]
	stp	x23, x24, [x11, #48]
	stp	x25, x26, [x11, #64]
	stp	x27, x28, [x11, #80]
	stp	x29, x30, [x11, #96]

	/* Enter EL0: jumps to address in ELR_EL1, with the stack pointer at SP_EL0.
	SPSR_EL1 is used to set the processor state for EL0; all three registers must be correctly
	configured before calling `enter_el0`. */
	eret

1:
	/* Return to the caller (uses address stored in x30, which is the return address saved when
	enter_el0 was first called) */
	ret

/* Restores every register enter_el0 saved, then branches directly to the saved resume address
(enter_el0's own `1:` label) -- not a `ret`, since this isn't returning from a call, it's jumping
into the middle of a function that's still (from the CPU's perspective) mid-execution, waiting at
that label. x30 is restored as part of this, so enter_el0's own subsequent `ret` correctly returns
to run_program's call site once resumed.

Called from sync_el0_handler (syscall.rs) for `exit` and for a caught segfault alike -- never
returns itself. Takes the exit status in w0 and leaves x0 alone (only x9-x11 and the registers being
restored are touched), so it reaches enter_el0's `ret` as the return value. */
.global resume_kernel
resume_kernel:

	/* Load the address of KERNEL_CTX into x11 */
	ldr	x11, =KERNEL_CTX

	/* Load SP, resume address, and callee-saved registers from KERNEL_CTX */
	ldp	x9, x10, [x11, #0]
	mov	sp, x9 /* Restore SP from x9 */
	ldp	x19, x20, [x11, #16]
	ldp	x21, x22, [x11, #32]
	ldp	x23, x24, [x11, #48]
	ldp	x25, x26, [x11, #64]
	ldp	x27, x28, [x11, #80]
	ldp	x29, x30, [x11, #96]
	br	x10 /* Branch to the saved resume address (x10) */
