//! The kernel side of `open`/`close`/`getdents`/`chmod` and the per-handle half of `read`/
//! `write`: turns a path from a user program into an open file on Stage 8's FAT filesystem.
//! `syscall/fd.rs` owns the small fd numbers a program sees; this module owns what they refer to.
//!
//! Paths are absolute or root-relative -- `bin/cat.exe` and `/bin/cat.exe` mean the same thing,
//! since there is no working directory until Stage 12's shell introduces one. Components are
//! matched exactly (case-sensitively, same as `find_entry_checked`); `.`/`..` aren't special, so they
//! simply fail to match anything.
//!
//! Writes are deliberately narrow, matching what `cp` needs: opening for write creates the file
//! if it's missing and always starts from offset 0, and `hadris-fat`'s `FileWriter` truncates
//! the file to whatever was actually written when it's finished (`close`). There's no append,
//! no seek, and no separate truncate flag.

use alloc::string::String;
use alloc::vec::Vec;

use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::sync::read::FileReader;
use hadris_fat::sync::write::FileWriter;
use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FatVolumeReadExt, FatVolumeWriteExt};

use super::blkio::{BlkIo, VOL};
use super::find_entry_checked;
use crate::static_ref;
use abi::errno::{EACCES, EBADF, EINVAL, EIO, EISDIR, EMFILE, ENOENT, ENOTDIR};
use abi::fs::{ATTR_EXEC, ATTR_READ_ONLY, ATTR_VOLUME_LABEL, DIRENT_SIZE, NAME_MAX};

/// The only attribute bits `chmod` may change: the two this project exposes as permissions.
const CHMOD_BITS: u8 = ATTR_EXEC | ATTR_READ_ONLY;

/// How many files/directories may be open at once, across every fd a program holds. The fd table
/// (`syscall/fd.rs`) is sized from this (it adds the three standard fds), so `open` fails with
/// `EMFILE` at exactly this many, whichever table would have run out first.
pub const MAX_OPEN_FILES: usize = 13;

type Dir = FatDir<'static, BlkIo>;

/// One directory entry as `getdents` reports it, captured when the directory was opened.
struct DirRec {
    name: String,
    size: u32,
    attrs: u8,
}

/// One open file, which may be a reader, a writer, or a directory.
enum OpenFile {
    /// A file opened for reading, represented by its `FileReader`.
    Reader(FileReader<'static, BlkIo>),

    /// A file opened for writing, represented by its `FileWriter`.
    Writer(FileWriter<'static, BlkIo>),

    /// A directory. `recs` holds the directory's entries as `DirRec`.
    Dir { recs: Vec<DirRec>, next: usize },
}

/// The table `fd.rs`'s `FileDescriptor::File(handle)` indexes into. A handle stays valid from
/// `open` until `close` (or the end of the launch -- see `close_all`).
///
/// SAFETY (every access, via `table`): single core, and every syscall runs with IRQs masked, so
/// nothing else can touch this while a call is in progress.
static mut OPEN_FILES: [Option<OpenFile>; MAX_OPEN_FILES] = [const { None }; MAX_OPEN_FILES];

/// Accessor for `OPEN_FILES`.
fn table() -> &'static mut [Option<OpenFile>; MAX_OPEN_FILES] {
    // SAFETY: see OPEN_FILES.
    unsafe { &mut *(&raw mut OPEN_FILES) }
}

/// Accessor for the FAT volume, `VOL`.
fn vol() -> &'static FatVolume<BlkIo> {
    // SAFETY: VOL is populated before any program can run (kernel_main) and never cleared.
    unsafe { static_ref!(VOL) }
}

/// Splits a path into its components, ignoring empty components caused by consecutive slashes.
fn components(path: &str) -> Vec<&str> {
    path.split('/').filter(|c| !c.is_empty()).collect()
}

/// Resolve a directory from the root, given a slice of path components: each one must name a
/// directory inside the previous. An empty slice resolves to the root itself.
fn resolve(dirs: &[&str]) -> Result<Dir, isize> {
    let mut dir = vol().root_dir();
    for name in dirs {
        let entry = find_entry_checked(&dir, name)?.ok_or(ENOENT)?;
        if !entry.is_directory() {
            return Err(ENOTDIR);
        }
        dir = dir.open_entry(&entry).map_err(|_| EIO)?;
    }
    Ok(dir)
}

/// Finds the file or directory entry at `path` (absolute or root-relative), without opening it --
/// what the launcher uses to locate a program. `EISDIR` for the root itself, which has no entry.
pub fn lookup(path: &str) -> Result<hadris_fat::sync::FileEntry, isize> {
    let comps = components(path);
    let (leaf, parents) = comps.split_last().ok_or(EISDIR)?;
    let parent = resolve(parents)?;
    find_entry_checked(&parent, leaf)?.ok_or(ENOENT)
}

/// Lists the contents of the given directory, returning a vector of `DirRec` entries. Skips
/// the `.` and `..` entries, as well as volume labels. Returns `EIO` on an I/O error.
fn list(dir: &Dir) -> Result<Vec<DirRec>, isize> {
    let mut recs = Vec::new();
    for item in dir.entries() {
        let DirectoryEntry::Entry(entry) = item.map_err(|_| EIO)?;
        let name = entry.name();
        let attrs = entry.attributes().bits();
        if name == "." || name == ".." || attrs & ATTR_VOLUME_LABEL != 0 {
            continue;
        }
        recs.push(DirRec {
            name: String::from(name),
            size: entry.len() as u32,
            attrs,
        });
    }
    Ok(recs)
}

/// Opens a file for reading. If the path points to a directory, it opens the directory instead,
/// e.g., for listing its contents via `getdents`.
fn open_read(path: &str) -> Result<OpenFile, isize> {
    let comps = components(path);
    let Some((leaf, parents)) = comps.split_last() else {
        // No components at all: the root directory itself.
        return Ok(OpenFile::Dir {
            recs: list(&vol().root_dir())?,
            next: 0,
        });
    };
    let parent = resolve(parents)?;
    let entry = find_entry_checked(&parent, leaf)?.ok_or(ENOENT)?;
    if entry.is_directory() {
        let dir = parent.open_entry(&entry).map_err(|_| EIO)?;
        Ok(OpenFile::Dir {
            recs: list(&dir)?,
            next: 0,
        })
    } else {
        Ok(OpenFile::Reader(vol().read_file(&entry).map_err(|_| EIO)?))
    }
}

/// Opens a file for writing. If the file does not exist, it is created. Returns an
/// `OpenFile::Writer` handle. If the path points to a directory, it returns `EISDIR`. If the file
/// is read-only, it returns `EACCES`.
fn open_write(path: &str) -> Result<OpenFile, isize> {
    let comps = components(path);
    let (leaf, parents) = comps.split_last().ok_or(EISDIR)?;
    let parent = resolve(parents)?;
    let entry = match find_entry_checked(&parent, leaf)? {
        Some(entry) => {
            if entry.is_directory() {
                return Err(EISDIR);
            }
            if entry.attributes().bits() & ATTR_READ_ONLY != 0 {
                return Err(EACCES);
            }
            entry
        }
        None => vol().create_file(&parent, leaf).map_err(|_| EIO)?,
    };
    // A second writer on the same file is rejected by hadris-fat itself, which surfaces here.
    Ok(OpenFile::Writer(vol().write_file(&entry).map_err(|_| EIO)?))
}

/// Opens `path` and returns its handle. Read mode opens a file, or a directory (for
/// `getdents`); write mode creates the file if needed and starts it from empty.
pub fn open(path: &str, write: bool) -> Result<usize, isize> {
    let slot = table().iter().position(Option::is_none).ok_or(EMFILE)?;
    let file = if write {
        open_write(path)?
    } else {
        open_read(path)?
    };
    table()[slot] = Some(file);
    Ok(slot)
}

/// Reads from the file represented by `handle` into `buf`. Returns the number of bytes read, EBADF
/// if the handle is not open for reading, EISDIR if the handle is a directory, or EIO on an I/O
/// error.
pub fn read(handle: usize, buf: &mut [u8]) -> isize {
    match table()[handle].as_mut() {
        Some(OpenFile::Reader(reader)) => match reader.read(buf) {
            Ok(n) => n as isize,
            Err(_) => EIO,
        },
        Some(OpenFile::Dir { .. }) => EISDIR,
        _ => EBADF,
    }
}

/// Writes `bytes` to the file represented by `handle`. Returns the number of bytes written, EBADF
/// if the handle is not open for writing, or EIO on an I/O error.
pub fn write(handle: usize, bytes: &[u8]) -> isize {
    match table()[handle].as_mut() {
        Some(OpenFile::Writer(writer)) => match writer.write(bytes) {
            Ok(n) => n as isize,
            Err(_) => EIO,
        },
        _ => EBADF,
    }
}

/// Batched reading of directory entries. Fills `buf` with as many directory entries as can fit in
/// one call. Returns the number of bytes written to `buf`. Moves the `next` index in the
/// referened `OpenFile::Dir` structure in `table()[handle]` accordingly.
pub fn getdents(handle: usize, buf: &mut [u8]) -> isize {
    let Some(OpenFile::Dir { recs, next }) = table()[handle].as_mut() else {
        // If the handle is not a directory, return `ENOTDIR`.
        return ENOTDIR;
    };

    let mut off = 0;
    while *next < recs.len() && off + DIRENT_SIZE <= buf.len() {
        let rec = &recs[*next];
        let name = rec.name.as_bytes();
        let n = name.len().min(NAME_MAX);

        // Each entry is a fixed-size `DIRENT_SIZE` byte array.
        let out = &mut buf[off..off + DIRENT_SIZE];

        // File size (4 bytes, little-endian)
        out[0..4].copy_from_slice(&rec.size.to_le_bytes());

        // Attributes (1 byte)
        out[4] = rec.attrs;

        // Name length (1 byte)
        out[5] = n as u8;

        // Name, null-padded to `NAME_MAX` bytes
        out[6..6 + n].copy_from_slice(&name[..n]);
        out[6 + n..].fill(0);

        off += DIRENT_SIZE;
        *next += 1;
    }
    off as isize
}

/// Closes `handle`. For a writer, this is also what commits the file's final size to disk.
pub fn close(handle: usize) -> isize {
    match table()[handle].take() {
        Some(OpenFile::Writer(writer)) => match writer.finish() {
            Ok(()) => 0,
            Err(_) => EIO,
        },
        Some(_) => 0,
        None => EBADF,
    }
}

/// Closes every open handle -- called between launches so a program that exits (or faults)
/// without closing its files still gets them committed, and can't leak handles to the next one.
pub fn close_all() {
    for handle in 0..MAX_OPEN_FILES {
        let _ = close(handle);
    }
}

/// Sets the `set` bits and clears the `clear` bits in `path`'s FAT attribute byte. Only
/// `CHMOD_BITS` may be named in either mask -- the directory/volume-label/etc. bits describe
/// what an entry *is*, not a permission.
pub fn chmod(path: &str, set: u8, clear: u8) -> isize {
    if (set | clear) & !CHMOD_BITS != 0 {
        return EINVAL;
    }
    let comps = components(path);
    let Some((leaf, parents)) = comps.split_last() else {
        return EINVAL; // the root directory has no attribute byte
    };
    let entry = match resolve(parents)
        .and_then(|parent| find_entry_checked(&parent, leaf)?.ok_or(ENOENT))
    {
        Ok(entry) => entry,
        Err(e) => return e,
    };
    let attrs = (entry.attributes().bits() | set) & !clear;
    match vol().set_attributes(&entry, DirEntryAttrFlags::from_bits_retain(attrs)) {
        Ok(()) => 0,
        Err(_) => EIO,
    }
}
