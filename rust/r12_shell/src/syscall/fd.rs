//! File descriptors for the syscalls in `syscall.rs`. There is one fd table, not one per
//! program: at most one program is ever resident (see `arch/mmu.rs`'s reasoning), so
//! `reset_for_launch` simply refills it with the three standard entries before each launch --
//! `0`/`1`/`2` are `Keyboard`/`Console`/`Console` -- and everything above them is handed out by
//! `open` and returned by `close`. A launcher that wants to redirect a standard fd (Stage 12)
//! rebinds an entry after that reset and before the program starts.
//!
//! `File(handle)` entries refer to `fs/files.rs`'s open-file table; `Console` and `Keyboard` have
//! no state of their own here.

use crate::console::{BG, FG};
use crate::fs::files;
use crate::keyboard::stdin;
use crate::platform::base_addresses::{USER_BASE, USER_SIZE};
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::uart_write;
use crate::static_mut_ref;
use abi::errno::{EBADF, EFAULT, EINVAL, EMFILE};
use abi::fs::{O_RDONLY, O_WRONLY};

/// How many fds a program may have open at once, the three standard ones included: every fd above
/// them refers to an open file (`files::MAX_OPEN_FILES` of them), so `open` fails with `EMFILE` at
/// exactly that many, whichever table would have run out first.
const MAX_FDS: usize = 3 + files::MAX_OPEN_FILES;

#[derive(Clone, Copy)]
enum FileDescriptor {
    Console,
    Keyboard,
    File(usize),
}

/// SAFETY (every access, via `table`): single core, and every syscall runs with IRQs masked, so
/// nothing else can touch the table while a call is in progress.
static mut FD_TABLE: [Option<FileDescriptor>; MAX_FDS] = [None; MAX_FDS];

/// Returns a mutable reference to the file descriptor table.
fn table() -> &'static mut [Option<FileDescriptor>; MAX_FDS] {
    // SAFETY: see FD_TABLE.
    unsafe { &mut *(&raw mut FD_TABLE) }
}

/// Prepares the fd table for a new program: drops any unread typed input and refills the standard
/// three entries. Nothing can still be open from the last program -- `end_launch` closed it all
/// when that program ended, on every path (exit or fault).
pub fn reset_for_launch() {
    stdin::reset();
    let table = table();
    *table = [None; MAX_FDS];
    table[0] = Some(FileDescriptor::Keyboard);
    table[1] = Some(FileDescriptor::Console);
    table[2] = Some(FileDescriptor::Console);
}

/// Closes whatever the program that just ended left open, committing any file it was still
/// writing -- called right after it exits or faults, not left to the next `reset_for_launch`.
pub fn end_launch() {
    files::close_all();
}

impl FileDescriptor {
    fn for_fd(fd: usize) -> Option<Self> {
        table().get(fd).copied().flatten()
    }

    /// Writes to a file descriptor. `Keyboard` isn't writable.
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
                uart_write(text.as_bytes());
                bytes.len() as isize
            }
            FileDescriptor::Keyboard => EBADF,
            FileDescriptor::File(handle) => files::write(handle, bytes),
        }
    }

    /// Reads from a file descriptor. `Console` isn't readable.
    fn read(self, buf: &mut [u8]) -> isize {
        match self {
            FileDescriptor::Keyboard => stdin::read(buf),
            FileDescriptor::Console => EBADF,
            FileDescriptor::File(handle) => files::read(handle, buf),
        }
    }
}

/// Bounds-checks `ptr`/`len` against the fixed user window before trusting
/// them: the MMU's `USER` attribute bit gates *EL0's* access, not EL1's, so
/// a buggy or malicious pointer aimed at kernel memory is otherwise still
/// readable/writable by the kernel and must be rejected in software.
///
/// The end is computed with `checked_add`: a `ptr` near `usize::MAX` would otherwise wrap around
/// to a small sum and pass the range check.
fn validate(ptr: usize, len: usize) -> bool {
    ptr >= USER_BASE
        && len <= USER_SIZE
        && ptr
            .checked_add(len)
            .is_some_and(|end| end <= USER_BASE + USER_SIZE)
}

/// Validates a user-supplied path and returns it as a `&str`: `EFAULT` if the pointer is bad,
/// `EINVAL` if the bytes aren't UTF-8 (paths here are `str`s, not arbitrary byte strings).
fn user_path(ptr: usize, len: usize) -> Result<&'static str, isize> {
    if !validate(ptr, len) {
        return Err(EFAULT);
    }
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    core::str::from_utf8(bytes).map_err(|_| EINVAL)
}

/// Writes to a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes written, or a negative error.
pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within the user window.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    fd.write(bytes)
}

/// Reads from a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes read (`0` at end of file), or a negative
/// error.
pub fn read(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within the user window.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    fd.read(buf)
}

/// Opens the file or directory at the user-space path `ptr`/`len`, returning the lowest free fd
/// number (never one of the standard three) or a negative error.
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    let path = match user_path(ptr, len) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let write = match flags {
        O_RDONLY => false,
        O_WRONLY => true,
        _ => return EINVAL,
    };
    let table = table();
    let Some(fd) = table
        .iter()
        .skip(3)
        .position(Option::is_none)
        .map(|i| i + 3)
    else {
        return EMFILE;
    };
    match files::open(path, write) {
        Ok(handle) => {
            table[fd] = Some(FileDescriptor::File(handle));
            fd as isize
        }
        Err(e) => e,
    }
}

/// Closes `fd`. Closing a standard fd is allowed (it just leaves that slot empty).
pub fn close(fd: usize) -> isize {
    let Some(entry) = table().get_mut(fd).and_then(Option::take) else {
        return EBADF;
    };
    match entry {
        FileDescriptor::File(handle) => files::close(handle),
        FileDescriptor::Console | FileDescriptor::Keyboard => 0,
    }
}

/// Reads the next batch of directory records from an fd opened on a directory -- see
/// `files::getdents` for the record layout.
pub fn getdents(fd: usize, ptr: usize, len: usize) -> isize {
    let Some(FileDescriptor::File(handle)) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within the user window.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    files::getdents(handle, buf)
}

/// Sets and clears permission bits on the file at the user-space path `ptr`/`len` -- see
/// `files::chmod` for which bits are allowed.
pub fn chmod(ptr: usize, len: usize, set: usize, clear: usize) -> isize {
    let path = match user_path(ptr, len) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let (Ok(set), Ok(clear)) = (u8::try_from(set), u8::try_from(clear)) else {
        return EINVAL;
    };
    files::chmod(path, set, clear)
}
