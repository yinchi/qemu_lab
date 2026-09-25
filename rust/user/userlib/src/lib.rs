//! Shared `no_std` runtime for every EL0 binary this roadmap builds from Stage 9 onward (the
//! programs in `../progs`, and later the editor). Five modules, all re-exported flat so programs
//! write `userlib::write`, `userlib::exit`, `userlib::entry!`, ...:
//!
//! - `syscall`: the raw `svc` and the `syscall!` macro -- the mechanism, crate-private.
//! - `io`: everything a program does with an fd or a path (`read`, `write`, `open`, `close`,
//!   `getdents`, `chmod`).
//! - `time`: the clock (`clock_gettime`, `time`).
//! - `memory`: the program break (`brk`); with the `heap` feature, `heap` is a global allocator on it.
//! - `process`: the program's own life -- entry stub and macros, `argc`/`argv`, `exit` and exit
//!   statuses, the panic handler.

#![no_std]

// Declared first, with `#[macro_use]`: `syscall!` is a plain `macro_rules!`, visible only to the
// modules that follow it.
#[macro_use]
mod syscall;
mod io;
mod memory;
mod process;
mod time;

#[cfg(feature = "heap")]
mod heap;

pub use io::*;
pub use memory::*;
pub use process::*;
pub use time::*;


// The syscall numbers live in the shared `abi` crate (the kernel uses the same ones); re-exported
// so `userlib::SYS_WRITE`, ... keep working.
pub use abi::time::CLOCK_REALTIME;
pub use abi::syscall::{
    SYS_BRK, SYS_CHMOD, SYS_CLOCK_GETTIME, SYS_CLOSE, SYS_EXIT, SYS_GETCWD, SYS_GETDENTS, SYS_IOCTL, SYS_MKDIRAT,
    SYS_NEWFSTATAT, SYS_OPEN, SYS_READ, SYS_REBOOT, SYS_RENAMEAT, SYS_UNLINKAT, SYS_WRITE,
};
