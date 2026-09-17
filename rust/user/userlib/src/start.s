.section ".text.boot"
.global _start

/* Every EL0 binary built against userlib shares this entry point: the
kernel's loader `eret`s straight into it (per r09_userspace's ELF loader),
with SP_EL0 already set to the top of the fixed user window. There is no
argc/argv setup yet (that's Stage 10) and no stack reservation needed here
-- the kernel owns the whole user window's layout and decides where the
stack lives independently of this binary's own sections.

`main` here is the `#[no_mangle] extern "C" fn main() -> !` the
`userlib::entry!` macro generates in each binary crate -- see lib.rs. */
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
