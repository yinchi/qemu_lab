/* AArch64 exception vector table: 16 entries, 0x80 bytes apart, 0x800 total.
   Row = where the exception came from, column = exception type. Three entries do real work:
   "Current EL, SPx, IRQ" (irq_el1h, offset 0x280, a device interrupt while the kernel itself
   runs), and the two "Lower EL, AArch64" entries Stage 9 populated -- sync_el0_64 (syscalls
   and segfaults from a running EL0 program) and irq_el0_64 (a device interrupt while that
   program runs). Every other entry still traps to unexpected_exception() so a bug is visible
   instead of silently corrupting execution. */

/** Aligns to a 128-byte boundary (2^7 = 128 or 0x80), then writes a branch instruction to
the given label. */
.macro ventry label
	.align 7
    /* Branch to the given label */
	b	\label
.endm

/*
SUB: subtract
STR: store register    STP: store pair
LDR: load register     LDP: load pair
MRS: move general <- system register    MSR: move system <- general register
*/

/* Save all general-purpose registers x0-x29 using `stp`, which stores two registers at a
time.  Then, load the exception return address (ELR_EL1) and saved program status register
(SPSR_EL1) into x21 and x22, respectively. Store x30 and x21 (ELR_EL1) to the stack.
Finally, store x22 (SPSR_EL1) to the stack using `str` (store a single register since there's
only one left).

Total: 31 registers saved to the stack, plus 2 more for ELR_EL1 and SPSR_EL1, for a total of
33 * 8 = 264 bytes.  We round up to 272 bytes to maintain a 16-byte stack alignment.

Called at the beginning of every exception going to EL1. Every exception ends one of three ways:

- `kernel_exit`, where `eret` restores the saved context and returns to the point of interruption
  (EL0 or EL1 depending on the exception).
- `resume_kernel` (a program's exit or fault): restores only SP and the callee-saved registers that
  `enter_el0` saved (see `arch/context.s`) and continues in `run_program`, abandoning this frame.
- `unexpected_exception`, which eventually calls `hang()` via the Rust panic handler, staying in
  EL1.
*/
.macro kernel_entry
	sub	sp, sp, #272
	stp	x0, x1, [sp, #16 * 0]
	stp	x2, x3, [sp, #16 * 1]
	stp	x4, x5, [sp, #16 * 2]
	stp	x6, x7, [sp, #16 * 3]
	stp	x8, x9, [sp, #16 * 4]
	stp	x10, x11, [sp, #16 * 5]
	stp	x12, x13, [sp, #16 * 6]
	stp	x14, x15, [sp, #16 * 7]
	stp	x16, x17, [sp, #16 * 8]
	stp	x18, x19, [sp, #16 * 9]
	stp	x20, x21, [sp, #16 * 10]
	stp	x22, x23, [sp, #16 * 11]
	stp	x24, x25, [sp, #16 * 12]
	stp	x26, x27, [sp, #16 * 13]
	stp	x28, x29, [sp, #16 * 14]
	mrs	x21, elr_el1
	mrs	x22, spsr_el1
	stp	x30, x21, [sp, #16 * 15]
	str	x22, [sp, #16 * 16]
.endm

/* Exact reverse of kernel_entry, restoring the 31 general-purpose registers,
two system registers, and the stack pointer. */
.macro kernel_exit
	ldr	x22, [sp, #16 * 16]
	ldp	x30, x21, [sp, #16 * 15]
	msr	spsr_el1, x22
	msr	elr_el1, x21
	ldp	x0, x1, [sp, #16 * 0]
	ldp	x2, x3, [sp, #16 * 1]
	ldp	x4, x5, [sp, #16 * 2]
	ldp	x6, x7, [sp, #16 * 3]
	ldp	x8, x9, [sp, #16 * 4]
	ldp	x10, x11, [sp, #16 * 5]
	ldp	x12, x13, [sp, #16 * 6]
	ldp	x14, x15, [sp, #16 * 7]
	ldp	x16, x17, [sp, #16 * 8]
	ldp	x18, x19, [sp, #16 * 9]
	ldp	x20, x21, [sp, #16 * 10]
	ldp	x22, x23, [sp, #16 * 11]
	ldp	x24, x25, [sp, #16 * 12]
	ldp	x26, x27, [sp, #16 * 13]
	ldp	x28, x29, [sp, #16 * 14]
	add	sp, sp, #272
	eret
.endm

/** Write the exception vector table, aligned to 2^11 = 2048 bytes (0x800).
Each entry is aligned to 2^7 bytes (0x80) due to the `ventry` macro. */
.align 11
.global vectors
vectors:
	ventry	sync_el1t
	ventry	irq_el1t
	ventry	fiq_el1t
	ventry	error_el1t

	ventry	sync_el1h
	ventry	irq_el1h
	ventry	fiq_el1h
	ventry	error_el1h

	ventry	sync_el0_64
	ventry	irq_el0_64
	ventry	fiq_el0_64
	ventry	error_el0_64

	ventry	sync_el0_32
	ventry	irq_el0_32
	ventry	fiq_el0_32
	ventry	error_el0_32

/** Exception vector table entries.  The value loaded into x0 indicates the type of exception,
matching the index in the vector table. Every entry branches to unexpected_exception(), where the
value in x0 becomes the argument `vector`, except for the three entries with real handlers below:

- irq_el1h/irq_el0_64 branch to irq_handler();
- sync_el0_64 branches to sync_el0_handler() with the trap frame's address in x0, becoming the
  handler's argument.
*/

sync_el1t:
	kernel_entry
	mov	x0, #0
	bl	unexpected_exception
irq_el1t:
	kernel_entry
	mov	x0, #1
	bl	unexpected_exception
fiq_el1t:
	kernel_entry
	mov	x0, #2
	bl	unexpected_exception
error_el1t:
	kernel_entry
	mov	x0, #3
	bl	unexpected_exception

sync_el1h:
	kernel_entry
	mov	x0, #4
	bl	unexpected_exception
irq_el1h:
	kernel_entry
	bl	irq_handler
	kernel_exit
fiq_el1h:
	kernel_entry
	mov	x0, #6
	bl	unexpected_exception
error_el1h:
	kernel_entry
	mov	x0, #7
	bl	unexpected_exception

sync_el0_64:
	kernel_entry
	mov	x0, sp
	bl	sync_el0_handler
	kernel_exit
irq_el0_64:
	kernel_entry
	bl	irq_handler
	kernel_exit
fiq_el0_64:
	kernel_entry
	mov	x0, #10
	bl	unexpected_exception
error_el0_64:
	kernel_entry
	mov	x0, #11
	bl	unexpected_exception

sync_el0_32:
	kernel_entry
	mov	x0, #12
	bl	unexpected_exception
irq_el0_32:
	kernel_entry
	mov	x0, #13
	bl	unexpected_exception
fiq_el0_32:
	kernel_entry
	mov	x0, #14
	bl	unexpected_exception
error_el0_32:
	kernel_entry
	mov	x0, #15
	bl	unexpected_exception
