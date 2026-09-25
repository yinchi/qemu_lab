//! File descriptors for the syscalls in `syscall.rs`. There is one fd table, not one per
//! program: at most one program is ever resident (see `arch/mmu.rs`'s reasoning), so
//! `reset_for_launch` simply refills it with the three standard entries before each launch --
//! `0`/`1`/`2` are `Keyboard`/`Console`/`Console` -- and everything above them is handed out by
//! `open` and returned by `close`. A launcher that wants to redirect a standard fd (Stage 12)
//! rebinds an entry after that reset and before the program starts.
//!
//! `File(file)` entries hold a reference to an open file (`fs/files.rs`'s `FileRef`); several fds, and the
//! shell's stream bindings, can refer to the same one, and it is closed when the last reference goes.
//! `Console` and `Keyboard` have no state of their own here.

use crate::console::utf8::Utf8Decoder;
use crate::console::{BG, FG};
use crate::exec::elf;
use crate::exec::shell_state::{self, Stdio};
use crate::fs::files::{self, FileRef};
use crate::keyboard::stdin;
use crate::platform::base_addresses::{USER_BASE, USER_SIZE};
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::{uart_clear_screen, uart_write};
use crate::static_mut_ref;
use abi::errno::{EBADF, EFAULT, EINVAL, EMFILE, ENOTTY, ERANGE};
use abi::fs::{AT_REMOVEDIR, O_APPEND, O_RDONLY, O_WRONLY, STAT_SIZE};
use abi::ioctl::CONSOLE_CLEAR;

/// How many fds a program may have open at once, the three standard ones included: every fd above
/// them refers to an open file (`files::MAX_OPEN_FILES` of them), so `open` fails with `EMFILE` at
/// exactly that many, whichever table would have run out first.
const MAX_FDS: usize = 3 + files::MAX_OPEN_FILES;

#[derive(Clone)]
enum FileDescriptor {
    Console,
    Keyboard,
    File(FileRef),
}

/// SAFETY (every access, via `table`): single core, and every syscall runs with IRQs masked, so
/// nothing else can touch the table while a call is in progress.
static mut FD_TABLE: [Option<FileDescriptor>; MAX_FDS] = [const { None }; MAX_FDS];

/// Returns a mutable reference to the file descriptor table.
#[allow(clippy::deref_addrof)]
fn table() -> &'static mut [Option<FileDescriptor>; MAX_FDS] {
    // SAFETY: see FD_TABLE.
    unsafe { &mut *(&raw mut FD_TABLE) }
}

/// Prepares the fd table for a new program: drops any unread typed input and refills the standard
/// three entries. Nothing can still be open from the last program -- `end_launch` emptied the table
/// when that program ended, on every path (exit or fault).
pub fn reset_for_launch() {
    stdin::reset();
    let table = table();
    *table = [const { None }; MAX_FDS];
    // Each standard stream is whatever the shell's current frame binds it to: a file shares the
    // frame's reference, so `2>&1` gives fds 1 and 2 the one open file.
    for (n, slot) in table.iter_mut().take(3).enumerate() {
        *slot = Some(match (shell_state::stdio(n), n) {
            (Stdio::File(file), _) => FileDescriptor::File(file),
            (Stdio::Default, 0) => FileDescriptor::Keyboard,
            (Stdio::Default, _) => FileDescriptor::Console,
        });
    }
}

/// Drops the fds of the program that just ended, which closes whatever only that program still held
/// and finishes any file it was still writing (a written file is only complete on disk once it's
/// closed). A file the shell also holds (a redirect target) stays open until the shell lets go.
/// Cleanup, not a save: nothing the program held only in memory is written out.
pub fn end_launch() {
    *table() = [const { None }; MAX_FDS];
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

    #[allow(clippy::deref_addrof)]
    fn flushes() -> &'static mut usize {
        // SAFETY: single core, IRQs masked in syscalls (see FD_TABLE).
        unsafe { &mut *(&raw mut FLUSHES) }
    }

    pub fn count_flush() {
        *flushes() += 1;
    }

    pub fn report_and_reset() {
        let n = core::mem::replace(flushes(), 0);
        // Save and restore the line-start flag around this line: the harness strips it from the
        // transcript entirely (see `TESTHOOK_LINE` in `test/harness.py`), so it must be invisible to
        // `uart_ensure_newline`'s bookkeeping too, or a real line right before it that did *not* end
        // in a newline would wrongly look like it already had one once this line is stripped back out.
        let was_at_line_start = crate::platform::uart::uart_at_line_start();
        uart_write(alloc::format!("[testhooks] console_flushes={n}\n").as_bytes());
        crate::platform::uart::set_uart_at_line_start(was_at_line_start);
    }
}

impl FileDescriptor {
    fn for_fd(fd: usize) -> Option<Self> {
        table().get(fd).cloned().flatten()
    }

    /// Writes to a file descriptor. `Keyboard` isn't writable.
    fn write(self, bytes: &[u8]) -> isize {
        match self {
            FileDescriptor::Console => {
                console_write(bytes);
                bytes.len() as isize
            }
            FileDescriptor::Keyboard => EBADF,
            FileDescriptor::File(file) => files::write(&file, bytes),
        }
    }

    /// Reads from a file descriptor. `Console` isn't readable.
    fn read(self, buf: &mut [u8]) -> isize {
        match self {
            FileDescriptor::Keyboard => stdin::read(buf),
            FileDescriptor::Console => EBADF,
            FileDescriptor::File(file) => files::read(&file, buf),
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
pub(super) fn validate(ptr: usize, len: usize, write: bool) -> bool {
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
/// number (never one of the standard three) or a negative error. `EMFILE` when no fd is free or
/// `files::MAX_OPEN_FILES` files are already open (the shell's redirect files count).
pub fn open(ptr: usize, len: usize, flags: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let path = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let append = flags & O_APPEND != 0;
    let write = match flags & !O_APPEND {
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
    match files::open(&path, write, append) {
        Ok(file) => {
            table[fd] = Some(FileDescriptor::File(file));
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

/// Copies the working directory's absolute path into the user buffer `ptr`/`len` (no terminating NUL) and
/// returns its length; `ERANGE` if the buffer is too small, `EFAULT` if it is not writable user memory.
pub fn getcwd(ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // writes a user buffer: clear PAN while it does
    let cwd = shell_state::cwd();
    if cwd.len() > len {
        return ERANGE;
    }
    if !validate(ptr, len, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, cwd.len()) };
    buf.copy_from_slice(cwd.as_bytes());
    cwd.len() as isize
}

/// Closes `fd`: the slot is emptied, and the file it held is really closed only if that was the last
/// reference to it (`files::close`). Closing a standard fd is allowed; under a redirect it releases just
/// this fd's reference, not the file the other fds and the shell still hold.
pub fn close(fd: usize) -> isize {
    let Some(entry) = table().get_mut(fd).and_then(Option::take) else {
        return EBADF;
    };
    match entry {
        FileDescriptor::File(file) => files::close(file),
        FileDescriptor::Console | FileDescriptor::Keyboard => 0,
    }
}

/// Reads the next batch of directory records from an fd opened on a directory -- see
/// `files::getdents` for the record layout.
pub fn getdents(fd: usize, ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let Some(FileDescriptor::File(file)) = FileDescriptor::for_fd(fd) else {
        return EBADF;
    };
    if !validate(ptr, len, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
    files::getdents(&file, buf)
}

/// Sets and clears permission bits on the file at the user-space path `ptr`/`len` -- see
/// `files::chmod` for which bits are allowed.
pub fn chmod(ptr: usize, len: usize, set: usize, clear: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let path = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let (Ok(set), Ok(clear)) = (u8::try_from(set), u8::try_from(clear)) else {
        return EINVAL;
    };
    files::chmod(&path, set, clear)
}

/// Creates an empty directory at the user-space path `ptr`/`len` -- see `files::mkdir`.
pub fn mkdir(ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let path = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    files::mkdir(&path)
}

/// Removes the file, or (`flags & AT_REMOVEDIR`) the empty directory, at the user-space path
/// `ptr`/`len` -- see `files::unlink`.
pub fn unlink(ptr: usize, len: usize, flags: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    if flags & !AT_REMOVEDIR != 0 {
        return EINVAL;
    }
    let path = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    files::unlink(&path, flags & AT_REMOVEDIR != 0)
}

/// Renames or moves the entry at the user-space path `old_ptr`/`old_len` to `new_ptr`/`new_len` --
/// see `files::rename`.
pub fn rename(old_ptr: usize, old_len: usize, new_ptr: usize, new_len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let old = match user_path(old_ptr, old_len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    let new = match user_path(new_ptr, new_len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    files::rename(&old, &new)
}

/// Writes the size, attributes, and timestamps of the user-space path `ptr`/`len` into the
/// `STAT_SIZE`-byte buffer `out_ptr` -- see `files::stat` for the source and `abi::fs::STAT_SIZE`
/// for the layout.
pub fn stat(ptr: usize, len: usize, out_ptr: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let path = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(path) => path,
        Err(e) => return e,
    };
    if !validate(out_ptr, STAT_SIZE, true) {
        return EFAULT;
    }
    let info = match files::stat(&path) {
        Ok(info) => info,
        Err(e) => return e,
    };
    // SAFETY: validated above to lie entirely within writable user memory.
    let buf = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut u8, STAT_SIZE) };
    buf[0..4].copy_from_slice(&info.size.to_le_bytes());
    buf[4] = info.attrs;
    buf[5..7].copy_from_slice(&info.created.date.to_le_bytes());
    buf[7..9].copy_from_slice(&info.created.time.to_le_bytes());
    buf[9] = info.created.time_tenth;
    buf[10..12].copy_from_slice(&info.modified.date.to_le_bytes());
    buf[12..14].copy_from_slice(&info.modified.time.to_le_bytes());
    buf[14..16].copy_from_slice(&info.accessed_date.to_le_bytes());
    0
}
