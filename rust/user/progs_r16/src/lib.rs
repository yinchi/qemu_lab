//! What the Stage 16 programs share: helpers that the user heap (`userlib`'s `heap` feature) made possible.
//! Before Stage 16 a program had no heap, so `progs::PathBuf` joined paths in a fixed `PATH_MAX` buffer (`None` on
//! overflow), directories were read a few records at a time while walking them, and a few programs kept fixed
//! arrays. These replace those stand-ins with the `String`s and `Vec`s they were standing in for.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use userlib::{DIRENT_SIZE, DirEnt, O_RDONLY, close, getdents, open};

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

/// Why a directory could not be listed: the `open` failed, or a `getdents` did (an fd that is not a directory,
/// for one). Kept apart because `ls` words them differently.
pub enum ReadDirError {
    Open(isize),
    Read(isize),
}

/// The whole listing of the directory `path`, in on-disk order, with the fd already closed -- so a caller that
/// recurses (`chmod -R`, `rm -r`) holds no descriptor while it does, and depth is not limited by the number of
/// files the kernel lets a program keep open.
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

impl ReadDirError {
    /// The errno either kind carries.
    pub fn errno(&self) -> isize {
        match *self {
            ReadDirError::Open(e) | ReadDirError::Read(e) => e,
        }
    }
}
