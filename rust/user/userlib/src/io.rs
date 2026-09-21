//! Everything a program does with a file descriptor or a path: `read`, `write`, `open`, `close`,
//! `getdents` (and decoding what it returns) and `chmod`. Each is a thin wrapper over one syscall
//! (see `syscall.rs`); the errors they return are negated errno values (`abi::errno`).
//!
//! The one addition is a small stdout buffer (`write_stdout`, `flush_stdout`): a `write!` makes one
//! `write` per fragment and every console `write` costs the kernel a display flush, so formatted
//! output to fd 1 is collected and sent together. Ordering against every other write is kept by
//! flushing it first (see `write`).

use abi::syscall::{SYS_CHMOD, SYS_CLOSE, SYS_GETDENTS, SYS_OPEN, SYS_READ, SYS_WRITE};

// `O_*` flags, directory-record layout and attribute bits: the definitions live in the shared `abi`
// crate (the kernel uses the same ones) and are re-exported here, so programs keep writing
// `userlib::O_RDONLY`, `userlib::ATTR_EXEC`, ... exactly as before.
pub use abi::fs::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, NAME_MAX, O_RDONLY, O_WRONLY,
};

/// Writes `buf` to the file descriptor `fd`. Returns the number of bytes
/// written, or a negative value on error (see the kernel's `syscall/fd.rs`
/// for what can fail and why). Anything still waiting in the stdout buffer is sent first, so
/// output appears in the order the program produced it whichever fds it used.
pub fn write(fd: usize, buf: &[u8]) -> isize {
    flush_stdout();
    write_raw(fd, buf)
}

fn write_raw(fd: usize, buf: &[u8]) -> isize {
    syscall!(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Capacity of the stdout buffer. Fragments larger than what is left flush it first; a fragment this
/// big or bigger skips it.
const STDOUT_CAPACITY: usize = 512;

struct StdoutBuffer {
    bytes: [u8; STDOUT_CAPACITY],
    len: usize,
}

/// Single program, single thread, and nothing in the kernel touches it.
static mut STDOUT_BUFFER: StdoutBuffer = StdoutBuffer {
    bytes: [0; STDOUT_CAPACITY],
    len: 0,
};

fn stdout_buffer() -> &'static mut StdoutBuffer {
    // SAFETY: see STDOUT_BUFFER; callers never hold the reference across a call back into this module.
    unsafe { &mut *(&raw mut STDOUT_BUFFER) }
}

/// Queues `bytes` for fd 1 (stdout). They are sent when the buffer fills, when a fragment
/// contains a newline, before any other `write`, before a `read` from fd 0, and by `exit`.
/// Errors are not reported (as with C's `stdout`): a failing fd 1 loses the text.
pub fn write_stdout(bytes: &[u8]) {
    if bytes.len() > STDOUT_CAPACITY - stdout_buffer().len {
        flush_stdout();
    }
    if bytes.len() >= STDOUT_CAPACITY {
        send_all(bytes);
        return;
    }
    let buffer = stdout_buffer();
    buffer.bytes[buffer.len..buffer.len + bytes.len()].copy_from_slice(bytes);
    buffer.len += bytes.len();
    if bytes.contains(&b'\n') {
        flush_stdout();
    }
}

/// Sends whatever is waiting in the stdout buffer. Called by `exit`, so a program never loses
/// its last output by ending normally; a fault (or being killed) does lose it, like C.
pub fn flush_stdout() {
    let buffer = stdout_buffer();
    let len = core::mem::take(&mut buffer.len);
    if len > 0 {
        // The buffer is reused by nothing during this call: `send_all` uses `write_raw`.
        send_all(&stdout_buffer().bytes[..len]);
    }
}

/// Sends all of `bytes` to fd 1, retrying after a short write; gives up on an error.
fn send_all(mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let n = write_raw(1, bytes);
        if n <= 0 {
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

/// Reads up to `buf.len()` bytes from the file descriptor `fd` into `buf`. Returns the number of
/// bytes read -- `0` at end of file -- or a negative value on error. From fd `0` (the keyboard)
/// this blocks until a whole line has been typed and returns that line, newline included, no
/// matter how large `buf` is.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    if fd == 0 {
        flush_stdout(); // a prompt written just before this must be on the screen while we wait
    }
    syscall!(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// Opens the file or directory at `path` (absolute, or relative to the root -- there is no
/// working directory yet) and returns its fd, or a negative error.
pub fn open(path: &str, flags: usize) -> isize {
    syscall!(SYS_OPEN, path.as_ptr() as usize, path.len(), flags)
}

/// Closes `fd`. For a file opened with `O_WRONLY` this is also what commits its final size to
/// disk, so a failure here means the write may not have landed.
pub fn close(fd: usize) -> isize {
    syscall!(SYS_CLOSE, fd)
}

/// Fills `buf` with as many whole `DIRENT_SIZE` records as fit from an fd opened on a directory,
/// picking up where the last call left off. Returns the number of bytes filled -- `0` once the
/// listing is exhausted -- or a negative error.
pub fn getdents(fd: usize, buf: &mut [u8]) -> isize {
    syscall!(SYS_GETDENTS, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// One decoded `getdents` record.
pub struct DirEnt<'a> {
    pub size: u32,
    pub attrs: u8,
    pub name: &'a str,
}

impl<'a> DirEnt<'a> {
    /// Decodes the record in `raw`, which must be exactly `DIRENT_SIZE` bytes. `None` if the
    /// name isn't valid UTF-8.
    pub fn parse(raw: &'a [u8]) -> Option<Self> {
        let name_len = raw[5] as usize;
        Some(Self {
            size: u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            attrs: raw[4],
            name: core::str::from_utf8(&raw[6..6 + name_len]).ok()?,
        })
    }
}

/// Sets the `set` bits and clears the `clear` bits of `path`'s attribute byte. Only
/// `ATTR_READ_ONLY` and `ATTR_EXEC` may be named. Returns `0`, or a negative error.
pub fn chmod(path: &str, set: u8, clear: u8) -> isize {
    syscall!(SYS_CHMOD, path.as_ptr() as usize, path.len(), set as usize, clear as usize)
}
