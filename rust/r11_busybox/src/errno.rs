//! Error return values for the syscalls in `syscall.rs`: a negated Linux errno number, the same
//! convention Linux's own syscall ABI uses (a small negative `isize` in `x0`). Linux's numbering
//! is borrowed for familiarity only -- see `syscall.rs`'s note on the syscall numbers -- and
//! `userlib`/`progs` decode them back into messages (`progs::errmsg`), so the two crates must
//! agree on the values below by convention, same as the syscall numbers.

pub const EIO: isize = -5;
pub const EBADF: isize = -9;
pub const EACCES: isize = -13;
pub const ENOENT: isize = -2;
pub const ENOTDIR: isize = -20;
pub const EISDIR: isize = -21;
pub const EINVAL: isize = -22;
pub const EMFILE: isize = -24;
