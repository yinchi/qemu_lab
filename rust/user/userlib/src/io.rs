//! Everything a program does with a file descriptor or a path: `read`, `write`, `open`, `close`,
//! `getdents` (and decoding what it returns) and `chmod`. Each is a thin wrapper over one syscall
//! (see `syscall.rs`); the errors they return are negated errno values (`abi::errno`).

use abi::syscall::{SYS_CHMOD, SYS_CLOSE, SYS_GETDENTS, SYS_OPEN, SYS_READ, SYS_WRITE};

// `O_*` flags, directory-record layout and attribute bits: the definitions live in the shared `abi`
// crate (the kernel uses the same ones) and are re-exported here, so programs keep writing
// `userlib::O_RDONLY`, `userlib::ATTR_EXEC`, ... exactly as before.
pub use abi::fs::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, NAME_MAX, O_RDONLY, O_WRONLY,
};

/// Writes `buf` to the file descriptor `fd`. Returns the number of bytes
/// written, or a negative value on error (see the kernel's `syscall/fd.rs`
/// for what can fail and why).
pub fn write(fd: usize, buf: &[u8]) -> isize {
    syscall!(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Reads up to `buf.len()` bytes from the file descriptor `fd` into `buf`. Returns the number of
/// bytes read -- `0` at end of file -- or a negative value on error. From fd `0` (the keyboard)
/// this blocks until a whole line has been typed and returns that line, newline included, no
/// matter how large `buf` is.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
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
