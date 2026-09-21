//! Shared `no_std` runtime for every EL0 binary this roadmap builds from Stage 9 onward (the
//! programs in `../progs`, and later the editor). Three modules, all re-exported flat so programs
//! write `userlib::write`, `userlib::exit`, `userlib::entry!`, ...:
//!
//! - `syscall`: the raw `svc` and the `syscall!` macro -- the mechanism, crate-private.
//! - `io`: everything a program does with an fd or a path (`read`, `write`, `open`, `close`,
//!   `getdents`, `chmod`).
//! - `process`: the program's own life -- entry stub and macros, `argc`/`argv`, `exit` and exit
//!   statuses, the panic handler.

#![no_std]

// Declared first, with `#[macro_use]`: `syscall!` is a plain `macro_rules!`, visible only to the
// modules that follow it.
#[macro_use]
mod syscall;
mod io;
mod process;

pub use io::*;
pub use process::*;

// The syscall numbers live in the shared `abi` crate (the kernel uses the same ones); re-exported
// so `userlib::SYS_WRITE`, ... keep working.
pub use abi::syscall::{
    SYS_CHMOD, SYS_CLOSE, SYS_EXIT, SYS_GETDENTS, SYS_OPEN, SYS_READ, SYS_WRITE,
};
