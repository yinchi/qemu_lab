//! The filesystem service, built on the block driver: `blkio` presents the device to `hadris-fat` as
//! a byte stream, `files` is the open files and path handling behind `open`/`getdents`/`chmod`,
//! and the helpers here look up directory entries and read whole files on the mounted volume.

pub mod blkio;
pub mod fattime;
pub mod files;
pub mod path;
pub mod rtc_time;

use abi::errno::EIO;
use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FatVolumeReadExt, FileEntry};

use blkio::BlkIo;

/// Finds the entry named exactly `name` (case-sensitive) in `dir`.
/// - Returns `Ok(None)` if there isn't one -- a typo is an ordinary outcome to report,
///   not a kernel bug
/// - Returns `Err(EIO)` if the directory can't be read.
///
/// What `files.rs` resolves every component of a path with.
pub fn find_entry_checked<'a>(
    dir: &FatDir<'a, BlkIo>,
    name: &str,
) -> Result<Option<FileEntry>, isize> {
    for item in dir.entries() {
        let DirectoryEntry::Entry(entry) = item.map_err(|_| EIO)?;
        if entry.name() == name {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Reads the whole file at the absolute path `path` on `vol`: `ENOENT` if any component is missing, `ENOTDIR`
/// if a directory component is a file, `EISDIR` if the last one is a directory. Takes the volume as an argument
/// (not the `VOL` static) so boot code can use it before the statics are set up; `files.rs` is what programs'
/// paths go through once they are.
pub fn read_path(vol: &FatVolume<BlkIo>, path: &str) -> Result<alloc::vec::Vec<u8>, isize> {
    use abi::errno::{EISDIR, ENOENT, ENOTDIR};
    let mut dir = vol.root_dir();
    let mut components = path.split('/').filter(|c| !c.is_empty()).peekable();
    while let Some(name) = components.next() {
        let entry = find_entry_checked(&dir, name)?.ok_or(ENOENT)?;
        if components.peek().is_some() {
            if !entry.is_directory() {
                return Err(ENOTDIR);
            }
            dir = dir.open_entry(&entry).map_err(|_| EIO)?;
        } else if entry.is_directory() {
            return Err(EISDIR);
        } else {
            return read_file_checked(vol, &entry);
        }
    }
    Err(EISDIR) // no components: the root
}

/// Reads a whole file into a freshly allocated `Vec<u8>`, or `Err(EIO)` if the filesystem can't
/// deliver it. The caller has already bounded the file's size (see `shell::launch`).
pub fn read_file_checked(
    vol: &FatVolume<BlkIo>,
    entry: &FileEntry,
) -> Result<alloc::vec::Vec<u8>, isize> {
    vol.read_file(entry)
        .map_err(|_| EIO)? // Error if can't open the file for reading
        .read_to_vec()
        .map_err(|_| EIO) // Error if can't read the file into a Vec
}
