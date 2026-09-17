//! File descriptor dispatch for the `write`/`read` syscalls. At most one
//! program is ever resident (see `mmu.rs`'s reasoning), and no redirection
//! exists yet, so the mapping from a small fd number to what it refers to
//! is fixed rather than a real per-process table -- `0`/`1`/`2` always mean
//! `Keyboard`/`Console`/`Console`. A real table, letting a launcher rebind
//! an entry before starting a program, is what Stage 12's `>`/`<`
//! redirection needs; nothing in this project exercises that yet.

use core::fmt::Write;

use crate::console::ConsoleWriter;
use crate::mmu::{USER_BASE, USER_SIZE};

/// This project's own error convention for a syscall -- a negative value,
/// not a claim about matching any particular Linux `errno`.
const EBADF: isize = -1;

#[derive(Clone, Copy)]
enum FileDescriptor {
    Console,
    Keyboard,
}

impl FileDescriptor {
    fn for_fd(fd: usize) -> Option<Self> {
        match fd {
            0 => Some(FileDescriptor::Keyboard),
            1 | 2 => Some(FileDescriptor::Console),
            _ => None,
        }
    }

    /// Writes to the console (both the GPU text console and the UART, for
    /// dual visibility). Writing to `Keyboard` is rejected outright, not
    /// silently accepted as a no-op: a real bug (a mixed-up fd number)
    /// should be visible as an error, not hidden behind output that simply
    /// never appears.
    fn write(self, bytes: &[u8]) -> isize {
        match self {
            FileDescriptor::Console => {
                let text = core::str::from_utf8(bytes).unwrap_or("<invalid utf8>");
                let _ = ConsoleWriter.write_str(text);
                bytes.len() as isize
            }
            FileDescriptor::Keyboard => EBADF,
        }
    }

    /// Reading from `Console` is rejected for the same reason writing to
    /// `Keyboard` is. `Keyboard`'s own real body -- draining a kernel-side
    /// buffer a keyboard IRQ handler fills -- is Stage 11's work; nothing
    /// in Stage 9 calls `read` at all.
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
