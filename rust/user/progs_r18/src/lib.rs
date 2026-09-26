//! What the Stage 18 programs share: the directory-walking helpers the overrides need (`join`, `read_dir`; the
//! same as `progs_r16`'s, copied rather than depended on because that crate drags the time-zone database into the
//! build), and the pure text helpers the programs use, kept in their own files so they are host-tested like the kernel's
//! pure modules (`human`, and the filters' helpers as they arrive).

#![no_std]

extern crate alloc;

pub mod countspec;
pub mod cutlist;
pub mod glob;
pub mod human;
pub mod sortkey;
pub mod textutil;
pub mod trset;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::errno::ENOMEM;
use progs::CHUNK;
use userlib::{DIRENT_SIZE, DirEnt, O_RDONLY, close, getdents, open, read};

/// `dir/name`, trimming one trailing `/` from `dir` first so joining under the root does not double it. There
/// is no length limit here (the kernel still refuses a path over `PATH_MAX`, when the program uses it).
pub fn join(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.strip_suffix('/').unwrap_or(dir))
}

/// One entry of a directory listing: what `getdents` reports.
pub struct Entry {
    pub name: String,
    pub size: u32,
    pub attrs: u8,
}

/// Why a directory could not be listed: the `open` failed, or a `getdents` did (an fd that is not a directory, for
/// one). Kept apart because `ls` words them differently.
pub enum ReadDirError {
    Open(isize),
    Read(isize),
}

impl ReadDirError {
    /// The errno either kind carries.
    pub fn errno(&self) -> isize {
        match *self {
            ReadDirError::Open(e) | ReadDirError::Read(e) => e,
        }
    }
}

/// The whole listing of the directory `path`, in on-disk order, with the fd already closed -- so a caller that
/// recurses holds no descriptor while it does, and depth is not limited by the number of files the kernel lets a
/// program keep open.
pub fn read_dir(path: &str) -> Result<Vec<Entry>, ReadDirError> {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return Err(ReadDirError::Open(fd));
    }
    let fd = fd as usize;
    let mut entries = Vec::new();
    let mut buf = [0u8; DIRENT_SIZE * 8];
    let result = loop {
        let n = getdents(fd, &mut buf);
        if n < 0 {
            break Err(ReadDirError::Read(n));
        }
        if n == 0 {
            break Ok(());
        }
        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            if let Some(ent) = DirEnt::parse(raw) {
                entries.push(Entry { name: String::from(ent.name), size: ent.size, attrs: ent.attrs });
            }
        }
    };
    close(fd);
    result.map(|()| entries)
}

/// Everything readable from `fd`, until end of file: the filters hold whole inputs in the user heap. `Err` is the negative
/// error from `read`, or `ENOMEM` if the heap cannot grow to hold it (reported, not a panic).
pub fn read_all(fd: usize) -> Result<Vec<u8>, isize> {
    let mut data = Vec::new();
    let mut chunk = [0u8; CHUNK];
    loop {
        let n = read(fd, &mut chunk);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(data);
        }
        data.try_reserve(n as usize).map_err(|_| ENOMEM)?;
        data.extend_from_slice(&chunk[..n as usize]);
    }
}
