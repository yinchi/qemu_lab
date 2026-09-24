//! A program's own life: its entry stub (`start.s`) and `entry!`/`entry_with_args!` macros, decoding
//! `argc`/`argv` (`Args`), and ending -- `exit`, exit statuses (`Termination`, `ExitCode`), and the
//! panic handler, which ends the program the same way.

use core::arch::asm;

use abi::syscall::{SYS_EXIT, SYS_REBOOT};

/// The `cmd` values `reboot` takes, re-exported so programs write `userlib::LINUX_REBOOT_CMD_POWER_OFF`.
pub use abi::reboot::{LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};

core::arch::global_asm!(include_str!("start.s"));

/// A user-program panic must not double-panic the kernel -- it terminates
/// the program the same way any other abnormal ending does, via `exit`.
/// `101` matches `std`'s own conventional exit code for a panicking Rust
/// process, for the same reason the `abi` crate borrows Linux's syscall numbers:
/// familiarity, not a functional dependency on anything upstream.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    exit(101)
}

/// Ends the calling program and returns control to the kernel -- all user programs must do this to
/// terminate correctly. `no_mangle` so that `start.s` can call it directly by symbol name;
/// this occurs in the fallback path if a program returns without calling `exit`, in which case
/// `start.s` will call `exit` with code 255.
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: i32) -> ! {
    crate::flush_stdout();
    syscall!(SYS_EXIT, code as usize);
    // `exit` never returns from the kernel's side either; loop defensively
    // in case it somehow does, rather than falling into whatever follows.
    loop {
        unsafe { asm!("wfe") }
    }
}

/// Powers off (`LINUX_REBOOT_CMD_POWER_OFF`) or restarts (`LINUX_REBOOT_CMD_RESTART`) the machine.
/// Never returns on success; returns a negative error (`EINVAL`) if `cmd` isn't recognized.
pub fn reboot(cmd: u32) -> isize {
    syscall!(SYS_REBOOT, cmd as usize)
}

/// Mirrors `std::process::Termination`, simplified to what a syscall's
/// scalar return value can actually carry: a program either finished with
/// nothing to report (`()`, exit code 0) or a specific exit code
/// (`ExitCode`).
pub trait Termination {
    /// Converts the implementing type into an integer exit code.
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
/// the shared runtime, rather than every consumer (`echo`, `cat`, `ls`, ..., and later the
/// editor) re-implementing C-string scanning and UTF-8 validation itself.
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
        // Only `_start` (start.s) calls this, with the registers the kernel set up; it is not an API
        // that could be handed an arbitrary pointer.
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        pub extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
            // SAFETY: `_start` (start.s) forwards these straight from the kernel's `eret`,
            // upholding `args`'s contract by construction.
            $crate::terminate($run(unsafe { $crate::args(argc, argv) }))
        }
    };
}
