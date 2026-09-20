//! The filesystem service, built on the block driver: `blkio` presents the device to `hadris-fat` as
//! a byte stream, `files` is the open-file table and path handling behind `open`/`getdents`/`chmod`,
//! and the helpers here look up directory entries and read whole files on the mounted volume.

pub mod blkio;
pub mod files;

use abi::errno::EIO;
use hadris_fat::sync::{DirectoryEntry, FatDir, FatVolume, FatVolumeReadExt, FileEntry};

use blkio::BlkIo;

/// Finds a named entry in `dir`, case-sensitively matching the display name `hadris-fat` derives
/// from the on-disk 8.3/LFN entry. `Ok(None)` on a miss -- a typo is an ordinary, expected outcome
/// to report, not a kernel bug -- and `Err(EIO)` if the directory can't be read. What `files.rs`
/// resolves every component of a path with.
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

/// `find_entry_checked` for boot-time lookups of files this project's own build put on the image
/// (`bin/`, `fonts/spleen.raw`), where a failure to read the directory is as fatal as a miss: both
/// come back as `None` for the caller to `expect`.
pub fn find_entry<'a>(dir: &FatDir<'a, BlkIo>, name: &str) -> Option<FileEntry> {
    find_entry_checked(dir, name).ok().flatten()
}

/// Reads a whole file into a freshly allocated `Vec<u8>`, or `Err(EIO)` if the filesystem can't
/// deliver it. The caller has already bounded the file's size (see `shell::launch`).
pub fn read_file_checked(
    vol: &FatVolume<BlkIo>,
    entry: &FileEntry,
) -> Result<alloc::vec::Vec<u8>, isize> {
    vol.read_file(entry)
        .map_err(|_| EIO)?
        .read_to_vec()
        .map_err(|_| EIO)
}

/// Reads a whole file into a freshly allocated `Vec<u8>` -- for boot-time files (the font) whose
/// absence or corruption is fatal; `shell::launch` uses `read_file_checked`.
pub fn read_file_to_vec(vol: &FatVolume<BlkIo>, entry: &FileEntry) -> alloc::vec::Vec<u8> {
    vol.read_file(entry)
        .expect("failed to open file for reading")
        .read_to_vec()
        .expect("failed to read file")
}
