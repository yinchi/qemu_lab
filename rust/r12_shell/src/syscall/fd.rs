//! File descriptors for the syscalls in `syscall.rs`. There is one fd table, not one per
//! program: at most one program is ever resident (see `arch/mmu.rs`'s reasoning), so
//! `reset_for_launch` simply refills it with the three standard entries before each launch --
//! `0`/`1`/`2` are `Keyboard`/`Console`/`Console` -- and everything above them is handed out by
//! `open` and returned by `close`. A launcher that wants to redirect a standard fd (Stage 12)
//! rebinds an entry after that reset and before the program starts.
//!
//! `File(handle)` entries refer to `fs/files.rs`'s open-file table; `Console` and `Keyboard` have
//! no state of their own here.

use crate::console::utf8::Utf8Decoder;
use crate::console::{BG, FG};
use crate::exec::elf;
use crate::fs::files;
use crate::keyboard::stdin;
use crate::platform::base_addresses::{USER_BASE, USER_SIZE};
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::{uart_clear_screen, uart_write};
use crate::static_mut_ref;
use abi::errno::{EBADF, EFAULT, EINVAL, EMFILE, ENOTTY};
use abi::fs::{O_RDONLY, O_WRONLY};
use abi::ioctl::CONSOLE_CLEAR;

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
#[allow(clippy::deref_addrof)]
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

/// Closes whatever the program that just ended left open, which finishes any file it was still
/// writing (a written file is only complete on disk once it's closed).
/// Cleanup, not a save: nothing the program held only in memory is written out.
pub fn end_launch() {
    files::close_all();
    // A character the program left half-written becomes one U+FFFD now, not the first byte of the
    // next program's output.
    console_finish_stream();
    #[cfg(feature = "testhooks")]
    testhooks::report_and_reset();
}

/// The bytes of a program's console output are UTF-8, but a `write` can end in the middle of a
/// character (`cat` sends 4096-byte chunks), so the decoder outlives each call.
/// SAFETY (every access): single core, and syscalls run with IRQs masked, so nothing else touches it.
static mut CONSOLE_DECODER: Utf8Decoder = Utf8Decoder::new();

/// Draws `bytes` (UTF-8, possibly ending mid-character) on the console at the cursor and flushes
/// the display once for the whole call, not per character; no UART mirroring.
fn console_draw(bytes: &[u8]) {
    // SAFETY: CONSOLE/GPU are populated well before the keyboard's GIC line is ever enabled in
    // kernel_main, and so before any program (the only way this is ever reached) could possibly
    // be running; CONSOLE_DECODER: see its declaration.
    unsafe {
        let console = static_mut_ref!(CONSOLE);
        #[allow(clippy::deref_addrof)]
        let decoder = &mut *(&raw mut CONSOLE_DECODER);
        for &byte in bytes {
            decoder.push(byte, |c| console.write_char(c, FG, BG));
        }
        static_mut_ref!(GPU).flush();
    }
    #[cfg(feature = "testhooks")]
    testhooks::count_flush();
}

/// A program's console output: drawn (see `console_draw`) and mirrored to the UART as the raw bytes
/// it sent, so a character split across two calls is intact on the serial transcript. Also used by
/// the fault path to show the segmentation-fault message.
pub fn console_write(bytes: &[u8]) {
    console_draw(bytes);
    uart_write(bytes);
}

/// Ends any character still incomplete (drawn as U+FFFD) and starts a fresh console line if the
/// cursor is mid-line -- the console half of `uart_ensure_newline`, for a message the kernel itself
/// is about to write.
pub fn console_start_line() {
    console_finish_stream();
    // SAFETY: see `console_draw`.
    unsafe {
        let console = static_mut_ref!(CONSOLE);
        if console.cursor().1 != 0 {
            console.write_char('\n', FG, BG);
        }
    }
}

/// Draws U+FFFD for an incomplete character, if there is one, and resets the decoder.
fn console_finish_stream() {
    // SAFETY: see `console_draw`.
    unsafe {
        let console = static_mut_ref!(CONSOLE);
        #[allow(clippy::deref_addrof)]
        let decoder = &mut *(&raw mut CONSOLE_DECODER);
        if decoder.is_pending() {
            decoder.finish(|c| console.write_char(c, FG, BG));
            static_mut_ref!(GPU).flush();
        }
    }
}

/// Test-only instrumentation (cargo feature `testhooks`, enabled for `just test`, off for
/// `just run`): counts GPU flushes done by console writes and prints the count on the UART when a
/// program ends, so the flush-count test is exact instead of timing-based. The harness strips these
/// lines from the transcripts it compares.
#[cfg(feature = "testhooks")]
mod testhooks {
    use crate::platform::uart::uart_write;

    static mut FLUSHES: usize = 0;

    pub fn count_flush() {
        // SAFETY: single core, IRQs masked in syscalls (see FD_TABLE).
        unsafe { *(&raw mut FLUSHES) += 1 };
    }

    pub fn report_and_reset() {
        // SAFETY: as above.
        let n = unsafe { core::mem::replace(&mut *(&raw mut FLUSHES), 0) };
        uart_write(alloc::format!("[testhooks] console_flushes={n}\n").as_bytes());
    }
}

impl FileDescriptor {
    fn for_fd(fd: usize) -> Option<Self> {
        table().get(fd).copied().flatten()
    }

    /// Writes to a file descriptor. `Keyboard` isn't writable.
    fn write(self, bytes: &[u8]) -> isize {
        match self {
            FileDescriptor::Console => {
                console_write(bytes);
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

/// Checks that the kernel may read (or, with `write`, write) the `len` bytes at `ptr` before it does:
/// they must lie in the user window *and* in pages the loader actually mapped with the needed
/// permission (`exec/usermem.rs`) -- not the guard below the stack, not the gap after the program,
/// and not a program's own read-only code when the kernel is about to write. The MMU's `USER`
/// attribute bit gates only *EL0's* access; the kernel, at EL1, faults on an unmapped or read-only
/// page too, and a kernel fault is a panic, so a bad pointer has to be refused here as `EFAULT`.
///
/// The end is computed with `checked_add` (in `allows`): a `ptr` near `usize::MAX` would otherwise
/// wrap around to a small sum and pass the range check.
fn validate(ptr: usize, len: usize, write: bool) -> bool {
    (USER_BASE..=USER_BASE + USER_SIZE).contains(&ptr) && elf::user_memory().allows(ptr, len, write)
}

/// Validates a user-supplied path and returns it as a `&str`: `EFAULT` if the pointer is bad,
/// `EINVAL` if the bytes aren't UTF-8 (paths here are `str`s, not arbitrary byte strings).
fn user_path(ptr: usize, len: usize) -> Result<&'static str, isize> {
    if !validate(ptr, len, false) {
        return Err(EFAULT);
    }
    // SAFETY: validated above to lie entirely within mapped user memory.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    core::str::from_utf8(bytes).map_err(|_| EINVAL)
}

/// Writes to a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes written, or a negative error.
pub fn write(fd: usize, ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len, false) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within mapped user memory.
    let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    fd.write(bytes)
}

/// Reads from a file descriptor from user space, after validating the pointer and length against
/// the fixed user window. Returns the number of bytes read (`0` at end of file), or a negative
/// error.
pub fn read(fd: usize, ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let Some(fd) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    fd.read(buf)
}

/// Opens the file or directory at the user-space path `ptr`/`len`, returning the lowest free fd
/// number (never one of the standard three) or a negative error.
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
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

/// Out-of-band control of what `fd` is open on (see `abi::ioctl` for the requests). Only the console
/// understands any today -- `CONSOLE_CLEAR` clears it, and the terminal on the other end of the UART
/// with it; any other request, or any other kind of fd, is `ENOTTY`; an fd that isn't open is `EBADF`.
pub fn ioctl(fd: usize, request: usize, _arg: usize) -> isize {
    match (FileDescriptor::for_fd(fd), request) {
        (None, _) => EBADF,
        (Some(FileDescriptor::Console), CONSOLE_CLEAR) => {
            // SAFETY: as `console_draw`.
            unsafe {
                static_mut_ref!(CONSOLE).clear(BG);
                static_mut_ref!(GPU).flush();
            }
            uart_clear_screen();
            0
        }
        _ => ENOTTY,
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
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let Some(FileDescriptor::File(handle)) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    files::getdents(handle, buf)
}

/// Sets and clears permission bits on the file at the user-space path `ptr`/`len` -- see
/// `files::chmod` for which bits are allowed.
pub fn chmod(ptr: usize, len: usize, set: usize, clear: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let path = match user_path(ptr, len) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let (Ok(set), Ok(clear)) = (u8::try_from(set), u8::try_from(clear)) else {
        return EINVAL;
    };
    files::chmod(path, set, clear)
}
