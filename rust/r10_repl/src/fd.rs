//! File descriptor dispatch for the `write`/`read` syscalls. At most one
//! program is ever resident (see `mmu.rs`'s reasoning), and no redirection
//! exists yet, so the mapping from a small fd number to what it refers to
//! is fixed rather than a real per-process table -- `0`/`1`/`2` always mean
//! `Keyboard`/`Console`/`Console`. A real table, letting a launcher rebind
//! an entry before starting a program, is what Stage 12's `>`/`<`
//! redirection needs; nothing in this project exercises that yet.

use crate::devices::{CONSOLE, GPU};
use crate::base_addresses::{USER_BASE, USER_SIZE};
use crate::{BG, FG, static_mut_ref};

/// Sentinel value indicating a bad file descriptor. Returned by `write`/`read`, which
/// return the number of bytes read or written, or `EBADF` on error.
const EBADF: isize = -1;

#[derive(Clone, Copy)]
enum FileDescriptor {
    Console,
    Keyboard,
}

impl FileDescriptor {

    /// Returns the `FileDescriptor` corresponding to a raw fd number, if any. Only `0`/`1`/`2` are
    /// valid for Stage 10, no disk support or stdin/out/err redirection yet (the kernel doesn't
    /// use file descriptors and thus does not have these limitations).
    fn for_fd(fd: usize) -> Option<Self> {
        match fd {
            0 => Some(FileDescriptor::Keyboard),
            1 | 2 => Some(FileDescriptor::Console),
            _ => None,
        }
    }

    /// Writes to a file descriptor. Only `Console` is writable; writing to `Keyboard` returns
    /// `EBADF`.
    fn write(self, bytes: &[u8]) -> isize {
        match self {
            FileDescriptor::Console => {
                let text = core::str::from_utf8(bytes).unwrap_or("<invalid utf8>");
                // SAFETY: CONSOLE/GPU are populated well before the keyboard's GIC line is ever
                // enabled in kernel_main, and so before any program (the only way this is ever
                // reached) could possibly be running.
                unsafe {
                    let console = static_mut_ref!(CONSOLE);
                    for c in text.chars() {
                        console.write_char(c, FG, BG);
                    }
                    static_mut_ref!(GPU).flush();
                }
                bytes.len() as isize
            }
            FileDescriptor::Keyboard => EBADF,
        }
    }

    /// Reads from a file descriptor. Only `Keyboard` is readable; reading from `Console` returns
    /// `EBADF`.
    /// 
    /// Reading from `Keyboard` returns 0 for now (no bytes read); an actual implementation is
    /// a TODO for Stage 11.
    fn read(self, _buf: &mut [u8]) -> isize {
        match self {
            FileDescriptor::Keyboard => 0,
            FileDescriptor::Console => EBADF,
        }
    }
}

/// Bounds-checks `ptr`/`len` against the fixed user window before trusting
/// them: the MMU's `USER` attribute bit gates *EL0's* access, not EL1's, so
/// a buggy or malicious pointer aimed at kernel memory is otherwise still
/// readable/writable by the kernel and must be rejected in software.
fn validate(ptr: usize, len: usize) -> bool {
    ptr >= USER_BASE && len <= USER_SIZE && ptr + len <= USER_BASE + USER_SIZE
}

/// Writes to a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes written, or `EBADF` on error.
pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EBADF;
    }
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    fd.write(bytes)
}

/// Reads from a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes read, or `EBADF` on error.
pub fn read(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EBADF;
    }
    // SAFETY: validated above to lie entirely within the user window.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    fd.read(buf)
}
