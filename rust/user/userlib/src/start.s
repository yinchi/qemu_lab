.section ".text.boot"
.global _start

/* Every EL0 binary built against userlib shares this entry point: the
kernel's loader `eret`s straight into it, with SP_EL0 already set -- to the
top of the fixed user window for a program taking no arguments (Stage 9's
hello/crash), or to wherever Stage 10's argv/stack setup left it, with
argc/argv in x0/x1 per the standard AArch64 calling convention (and, from
Stage 17, envp in x2, the way Linux's `main(argc, argv, envp)` gets it), for
one that does. No stack reservation is needed here either way -- the kernel
owns the whole user window's layout and decides where the stack lives
independently of this binary's own sections.

`bl main` forwards x0-x2 unchanged, since nothing above it touches those
registers -- whether `main` actually reads them as argc/argv depends only
on which of `userlib::entry!`/`entry_with_args!` (see lib.rs) generated
this crate's `main`: a 0-arg one simply never reads them, a 2-arg one
reads x0/x1, and `entry_with_env!`'s 3-arg one reads x2 as well. */
_start:
	bl	main

	/* `main` is declared `-> !` and must never return; reaching here means
	it violated that contract (a bug -- unsafe code or a UB path, since
	well-typed safe Rust can't fall off the end of a `-> !` function).
	Rather than hang the whole system silently, fall back to `exit` with a
	sentinel code so the kernel still regains control and the failure is at
	least observable -- the same preference for loud, visible failure over
	an invisible hang used elsewhere in this project (e.g. the MMU's
	segfault handling). Tail call (`b`, not `bl`): `exit` itself never
	returns, so there is no return address worth preserving. */
	mov	w0, #255
	b	exit
