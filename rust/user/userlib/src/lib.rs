//! Shared `no_std` runtime for every EL0 binary this roadmap builds from
//! Stage 9 onward (`hello`/`crash` now; `echo`/`cat`/`ls`/`cp`/the editor in
//! Stages 10-13): the entry stub (`start.s`), the raw syscall wrapper, and
//! the `write`/`read`/`exit` syscalls themselves, all built against the
//! standard AArch64/Linux calling convention -- syscall number in `x8`, up
//! to 6 args in `x0`-`x5`, return value in `x0` -- the same shape
//! `ROADMAP.md`'s Stage 9 assumes, not invented here.

#![no_std]

use core::arch::asm;

core::arch::global_asm!(include_str!("start.s"));

/// A user-program panic must not double-panic the kernel -- it terminates
/// the program the same way any other abnormal ending does, via `exit`.
/// `101` matches `std`'s own conventional exit code for a panicking Rust
/// process, for the same reason borrowing Linux's syscall numbers above
/// does: familiarity, not a functional dependency on anything upstream.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(101)
}

/// Syscall numbers. These are Linux's real aarch64 values (verified against
/// `include/uapi/asm-generic/unistd.h`), borrowed for familiarity only --
/// this project makes no other Linux-ABI-compatibility claim. In
/// particular: `SYS_EXIT` is plain `exit` (93), the single-thread-exit
/// syscall, not `exit_group` (94, what glibc's `exit()` actually calls to
/// tear down every thread in a process) -- irrelevant here since this
/// project has no threads, but worth naming so the choice reads as
/// deliberate rather than a mistake. Argument meaning, error/`errno`
/// conventions, and every syscall this project doesn't implement are all
/// our own design, not Linux's. Must stay in sync with the kernel's own
/// copy of these constants (r09_userspace's syscall dispatch) by
/// convention, since the two crates don't share a dependency.
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;

/// Issues `svc #0` with `nr` in `x8` and the full `x0`-`x5` argument width
/// this project's calling convention allows. Callers go through the `syscall!`
/// macro below rather than this directly, so they only ever write the
/// arguments a given syscall actually uses. The kernel's `kernel_entry`/
/// `kernel_exit` trampoline saves and restores every GPR across the trap,
/// so no clobber list beyond `x0`'s own input/output role is needed here --
/// unlike a real Linux syscall, nothing else this call touches is at risk
/// of being clobbered by the callee.
#[inline(always)]
unsafe fn syscall(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "svc #0",
            in("x8") nr,
            inout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
        );
    }
    ret
}

/// Pads a call out to `syscall`'s full 6-argument form with trailing
/// zeros, so callers only write the arguments a given syscall actually
/// uses -- `syscall!(SYS_EXIT, code)` instead of spelling out
/// `syscall(SYS_EXIT, code, 0, 0, 0, 0, 0)`. Also encapsulates the
/// `unsafe` block: every use in this crate is a plain scalar or a pointer
/// already derived from a safe slice, with no further safety obligation
/// beyond what `syscall` itself documents.
macro_rules! syscall {
    ($nr:expr) => {
        unsafe { syscall($nr, 0, 0, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr) => {
        unsafe { syscall($nr, $a0, 0, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr) => {
        unsafe { syscall($nr, $a0, $a1, 0, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr) => {
        unsafe { syscall($nr, $a0, $a1, $a2, 0, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr) => {
        unsafe { syscall($nr, $a0, $a1, $a2, $a3, 0, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr) => {
        unsafe { syscall($nr, $a0, $a1, $a2, $a3, $a4, 0) }
    };
    ($nr:expr, $a0:expr, $a1:expr, $a2:expr, $a3:expr, $a4:expr, $a5:expr) => {
        unsafe { syscall($nr, $a0, $a1, $a2, $a3, $a4, $a5) }
    };
}

/// Writes `buf` to the file descriptor `fd`. Returns the number of bytes
/// written, or a negative value on error (see r09_userspace's
/// `FileDescriptor` dispatch for what can fail and why).
pub fn write(fd: usize, buf: &[u8]) -> isize {
    syscall!(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Reads up to `buf.len()` bytes from the file descriptor `fd` into `buf`.
/// Stage 9 never calls this -- the kernel-side handler is a stub until
/// Stage 11.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// Ends the calling program and returns control to the kernel -- the only
/// sanctioned way for a user program to terminate (see `start.s`'s doc
/// comment for what happens if a program returns from `main` without
/// calling this). `#[no_mangle]` so `start.s`'s fallback path can tail-call
/// it directly by symbol name, the same way `entry!`-generated `main`
/// functions call it via `terminate`.
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: i32) -> ! {
    syscall!(SYS_EXIT, code as usize);
    // `exit` never returns from the kernel's side either; loop defensively
    // in case it somehow does, rather than falling into whatever follows.
    loop {
        unsafe { asm!("wfe") }
    }
}

/// Mirrors `std::process::Termination`, simplified to what a syscall's
/// scalar return value can actually carry: a program either finished with
/// nothing to report (`()`, exit code 0) or a specific exit code
/// (`ExitCode`).
pub trait Termination {
    fn report(self) -> i32;
}

impl Termination for () {
    fn report(self) -> i32 {
        0
    }
}

/// A specific process exit code, in the same 0-255 range a real Unix exit
/// status uses.
pub struct ExitCode(pub u8);

impl Termination for ExitCode {
    fn report(self) -> i32 {
        self.0 as i32
    }
}

/// Turns whatever a program's own entry function returned into the actual
/// `exit` syscall. Called by the `entry!`-generated `main` wrapper, not
/// directly by program code.
pub fn terminate<T: Termination>(result: T) -> ! {
    exit(result.report())
}

/// Generates the `#[no_mangle] extern "C" fn main() -> !` that `start.s`'s
/// `bl main` branches to, wired to call `$run` and pass its result through
/// `terminate`. `$run` may return `()` (exit code 0) or `ExitCode` (a
/// specific code) -- anything else implementing `Termination`.
///
/// One invocation of this per binary crate -- `userlib::entry!(run);` --
/// replaces what would otherwise be a hand-written wrapper duplicated
/// across every EL0 program this roadmap builds.
///
/// For a program that wants its command-line arguments, use `entry_with_args!` instead --
/// deliberately a separate macro rather than a change to this one, since Stage 9's `hello`/
/// `crash` already depend on `$run` taking no arguments and are done, not to be revisited.
#[macro_export]
macro_rules! entry {
    ($run:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn main() -> ! {
            $crate::terminate($run())
        }
    };
}

/// One argument in an `Args` (`argv[0]` is the program's own name, same C convention) --
/// `argc`/`argv` themselves, as `_start` receives them in `x0`/`x1`, aren't Rust-safe to hand a
/// program directly: this decodes them into an ordinary, `Copy`, single-pass iterator once, in
/// the shared runtime, rather than every consumer (`echo` now; `cat`/`ls`/`cp`/the editor in
/// Stages 11/13) re-implementing C-string scanning and UTF-8 validation itself.
#[derive(Clone, Copy)]
pub struct Args {
    argv: *const *const u8,
    remaining: usize,
}

impl Iterator for Args {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        if self.remaining == 0 {
            return None;
        }
        // SAFETY: see `args`'s doc comment -- the only way to construct an `Args` is through
        // it, so its contract already holds for every entry this reads.
        unsafe {
            let ptr = *self.argv;
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let bytes = core::slice::from_raw_parts(ptr, len);
            self.argv = self.argv.add(1);
            self.remaining -= 1;
            // The kernel only ever writes argument strings it got from its own UTF-8 `str`
            // buffer (see `ROADMAP.md`'s Stage 10 section) -- invalid UTF-8 here would mean
            // that contract was already broken, not something to paper over by silently ending
            // the iteration early.
            Some(core::str::from_utf8(bytes).expect("argv entry was not valid UTF-8"))
        }
    }
}

/// Decodes the raw `argc`/`argv` `_start` received into an `Args` iterator.
///
/// # Safety
/// `argv` must point to an array of exactly `argc` valid pointers, each to a NUL-terminated,
/// UTF-8 byte sequence, all still mapped for as long as the returned `Args` is used -- true for
/// whatever `_start` forwards directly from the kernel's own `eret`, never something a program
/// should construct itself.
pub unsafe fn args(argc: usize, argv: *const *const u8) -> Args {
    Args {
        argv,
        remaining: argc,
    }
}

/// Like `entry!`, but for a program that wants its command-line arguments: generates the
/// `#[no_mangle] extern "C" fn main(argc: usize, argv: *const *const u8) -> !` that `start.s`'s
/// `bl main` branches to (forwarding `x0`/`x1` unchanged -- see `start.s`'s doc comment), decodes
/// them via `args`, and calls `$run` with the resulting `Args`.
#[macro_export]
macro_rules! entry_with_args {
    ($run:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
            // SAFETY: `_start` (start.s) forwards these straight from the kernel's `eret`,
            // upholding `args`'s contract by construction.
            $crate::terminate($run(unsafe { $crate::args(argc, argv) }))
        }
    };
}
