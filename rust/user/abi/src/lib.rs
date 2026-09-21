//! The ABI shared by the kernel and every EL0 program -- one definition, instead of a copy on each
//! side kept in sync by convention. Four modules, used by their full path (`abi::errno::ENOENT`,
//! `abi::syscall::SYS_WRITE`, `abi::fs::ATTR_EXEC`, `abi::ioctl::CONSOLE_CLEAR`):
//!
//! - [`syscall`]: the syscall numbers.
//! - [`errno`]: the error values a syscall returns, and `errmsg` to turn one into text.
//! - [`ioctl`]: the request codes `SYS_IOCTL` takes.
//! - [`fs`]: the fixed layouts and flags around files -- `open` flags, the `getdents` record, the
//!   FAT attribute bits.
//!
//! Everything here is borrowed from Linux's aarch64 ABI *for familiarity only* -- this project makes
//! no other Linux-compatibility claim. Syscall numbers are Linux's real ones; errors are negated
//! Linux errno values returned in `x0`, so any negative `isize` is an error and the magnitude says
//! which; argument meanings and which syscalls exist at all are this project's own design.
//!
//! Values are never renumbered: a stage's kernel and its programs must keep working against the
//! copies every earlier stage compiled in (`r09`-`r11` predate this crate and have their own copies
//! of the same numbers -- each module's tests pin them).

#![no_std]

pub mod errno;
pub mod fs;
pub mod ioctl;
pub mod syscall;
