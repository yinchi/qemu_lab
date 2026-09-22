//! The filesystem service, built on the block driver: `blkio` presents the device to `hadris-fat` as
//! a byte stream, `files` is the open-file table and path handling behind `open`/`getdents`/`chmod`,
//! and the helpers here look up directory entries and read whole files on the mounted volume.

pub mod blkio;
pub mod files;
pub mod path;

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
