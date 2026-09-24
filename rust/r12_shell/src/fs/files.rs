//! The kernel side of `open`/`close`/`getdents`/`chmod` and the per-handle half of `read`/
//! `write`: turns a path from a user program into an open file on Stage 8's FAT filesystem.
//! `syscall/fd.rs` owns the small fd numbers a program sees; this module owns what they refer to.
//!
//! The paths this module takes are absolute and normalized (`fs::path::abspath` produces them from what a
//! user typed and the working directory): `/`, or `/` and components. Components are matched exactly
//! (case-sensitively, same as `find_entry_checked`); `.`/`..` never appear, having been resolved
//! lexically already.
//!
//! Opening for write creates the file if it's missing; it starts from offset 0 (truncating) unless
//! `append` positions it at the file's current end instead (`hadris-fat`'s `FileWriter::new_append`).
//! Either way, `close` (`FileWriter::finish`) is what commits the final size to disk. There's no
//! seek, and no separate truncate-without-writing flag.

use alloc::string::String;
use alloc::vec::Vec;

use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::sync::read::FileReader;
use hadris_fat::sync::write::FileWriter;
use hadris_fat::sync::{DirectoryEntry, FatDateTime, FatDir, FatVolume, FatVolumeReadExt, FatVolumeWriteExt};

use super::blkio::{BlkIo, VOL};
use super::find_entry_checked;
use crate::static_ref;
use abi::errno::{EACCES, EBADF, EEXIST, EINVAL, EIO, EISDIR, EMFILE, ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY};
use abi::fs::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, ATTR_VOLUME_LABEL, DIRENT_SIZE, NAME_MAX};

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

/// Whether each slot was opened by the shell itself (a redirect target), rather than by the program
/// that is about to run. `close_all` skips these -- the shell closes its own handles when the
/// redirected command (or, from Step 9, scope) they belong to is done with them, which can be well
/// after the one program that happened to run under it has already exited.
///
/// SAFETY: as `OPEN_FILES`.
static mut SHELL_OWNED: [bool; MAX_OPEN_FILES] = [false; MAX_OPEN_FILES];

/// Accessor for `OPEN_FILES`.
#[allow(clippy::deref_addrof)]
fn table() -> &'static mut [Option<OpenFile>; MAX_OPEN_FILES] {
    // SAFETY: see OPEN_FILES.
    unsafe { &mut *(&raw mut OPEN_FILES) }
}

/// Accessor for `SHELL_OWNED`.
#[allow(clippy::deref_addrof)]
fn shell_owned() -> &'static mut [bool; MAX_OPEN_FILES] {
    // SAFETY: see SHELL_OWNED.
    unsafe { &mut *(&raw mut SHELL_OWNED) }
}

/// Marks `handle` -- just returned by `open` -- as one the shell opened for a redirect rather than
/// one the about-to-run program opened for itself, so `close_all` leaves it alone.
pub fn mark_shell_owned(handle: usize) {
    shell_owned()[handle] = true;
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

/// Whether `path` names a directory that exists: `Ok(())`, or `ENOENT`/`ENOTDIR`/`EIO`. The root is one.
/// What `cd` checks before it moves.
pub fn check_directory(path: &str) -> Result<(), isize> {
    resolve(&components(path)).map(|_| ())
}

/// Finds the file or directory entry at `path`, without opening it --
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

/// Opens a file for writing. If the file does not exist, it is created. `append` positions the
/// writer at the file's current end instead of truncating it. Returns an `OpenFile::Writer` handle.
/// If the path points to a directory, it returns `EISDIR`. If the file is read-only, it returns
/// `EACCES`.
fn open_write(path: &str, append: bool) -> Result<OpenFile, isize> {
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
    let writer = if append {
        FileWriter::new_append(vol(), &entry).map_err(|_| EIO)?
    } else {
        vol().write_file(&entry).map_err(|_| EIO)?
    };
    Ok(OpenFile::Writer(writer))
}

/// Opens `path` and returns its handle. Read mode (`write: false`) opens a file, or a directory (for
/// `getdents`); write mode creates the file if needed, starting it empty or (`append`) from its
/// current end. `append` is ignored in read mode.
pub fn open(path: &str, write: bool, append: bool) -> Result<usize, isize> {
    let slot = table().iter().position(Option::is_none).ok_or(EMFILE)?;
    let file = if write {
        open_write(path, append)?
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

/// Closes `handle`. For a writer, this is also what commits the file's final size to disk. Clears
/// the shell-owned mark too, so whatever `open` next hands out this slot starts unmarked.
pub fn close(handle: usize) -> isize {
    shell_owned()[handle] = false;
    match table()[handle].take() {
        Some(OpenFile::Writer(writer)) => match writer.finish() {
            Ok(()) => 0,
            Err(_) => EIO,
        },
        Some(_) => 0,
        None => EBADF,
    }
}

/// Closes every open handle the program that just exited (or faulted) might have left open --
/// shell-owned handles (see `mark_shell_owned`) are skipped, since those belong to the shell's own
/// redirects, which can outlive the one program that happened to run under them.
pub fn close_all() {
    for handle in 0..MAX_OPEN_FILES {
        if !shell_owned()[handle] {
            let _ = close(handle);
        }
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

/// Maps a `hadris-fat` write-path error to an errno. Everything not named here (`NotAFile`,
/// `NotADirectory`, `EntryNotFound`, `StaleEntry`, `ClusterLoop`, `CorruptFilesystem`, ...) falls
/// back to `EIO`, matching how every other write path in this file already reports an unexpected
/// `hadris-fat` failure.
fn map_fat_err(e: hadris_fat::Error) -> isize {
    use hadris_fat::Error;
    match e {
        Error::AlreadyExists => EEXIST,
        Error::DirectoryNotEmpty => ENOTEMPTY,
        Error::InvalidPath | Error::InvalidFilename => EINVAL,
        Error::NoFreeSpace | Error::DirectoryFull => ENOSPC,
        _ => EIO,
    }
}

/// Creates an empty directory at `path`. `path`'s parent must already exist; `path` itself must
/// not.
pub fn mkdir(path: &str) -> isize {
    let comps = components(path);
    let Some((leaf, parents)) = comps.split_last() else {
        return EEXIST; // "/" always exists
    };
    let parent = match resolve(parents) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match vol().create_dir(&parent, leaf) {
        Ok(_) => 0,
        Err(e) => map_fat_err(e),
    }
}

/// Removes the file, or (`remove_dir`) the empty directory, at `path`. Deletion is governed by the
/// containing directory alone -- deliberately no read-only check here, matching real POSIX unlink
/// semantics for a privileged process; the read-only bit still gates `open_write`, just not this.
pub fn unlink(path: &str, remove_dir: bool) -> isize {
    let comps = components(path);
    let Some((leaf, parents)) = comps.split_last() else {
        return EINVAL; // can't unlink "/"
    };
    let parent = match resolve(parents) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let entry = match find_entry_checked(&parent, leaf) {
        Ok(Some(e)) => e,
        Ok(None) => return ENOENT,
        Err(e) => return e,
    };
    match (entry.is_directory(), remove_dir) {
        (true, false) => return EISDIR,
        (false, true) => return ENOTDIR,
        _ => {}
    }
    match vol().delete(&entry) {
        Ok(()) => 0,
        Err(e) => map_fat_err(e),
    }
}

/// Renames or moves the entry at `old` to `new`, within the same volume. Thin, literal wrapper
/// over `hadris-fat`'s `rename`: refuses if `new` already names something (`EEXIST`), regardless of
/// its type, and refuses moving a directory into its own descendant (`EINVAL`, from `InvalidPath`).
/// Deliberately does not implement "move into an existing directory" or "replace an existing file"
/// -- those are `mv`(1) behaviors, layered in userspace on top of `stat` + this + `unlink`.
pub fn rename(old: &str, new: &str) -> isize {
    let old_entry = match lookup(old) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let new_comps = components(new);
    let Some((new_leaf, new_parents)) = new_comps.split_last() else {
        return EISDIR; // can't rename onto "/"
    };
    let new_parent = match resolve(new_parents) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match vol().rename(&old_entry, &new_parent, new_leaf) {
        Ok(_) => 0,
        Err(e) => map_fat_err(e),
    }
}

/// Every field a FAT directory entry actually stores, as `stat` reports it: raw, packed FAT
/// date/time (see `abi::fs::STAT_SIZE`'s doc comment), not calendar values -- unpacking is left to
/// whichever program displays them.
pub struct StatInfo {
    pub size: u32,
    pub attrs: u8,
    pub created: FatDateTime,
    pub modified: FatDateTime,
    pub accessed_date: u16,
}

/// Reads `path`'s size, attributes, and timestamps. The root has no directory entry of its own, so
/// it's special-cased: size 0, `ATTR_DIRECTORY`, and the FAT epoch for every timestamp.
pub fn stat(path: &str) -> Result<StatInfo, isize> {
    if components(path).is_empty() {
        return Ok(StatInfo {
            size: 0,
            attrs: ATTR_DIRECTORY,
            created: FatDateTime::EPOCH,
            modified: FatDateTime::EPOCH,
            accessed_date: FatDateTime::EPOCH.date,
        });
    }
    let entry = lookup(path)?;
    Ok(StatInfo {
        size: entry.len() as u32,
        attrs: entry.attributes().bits(),
        created: entry.created(),
        modified: entry.modified(),
        accessed_date: entry.accessed_date(),
    })
}
