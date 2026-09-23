//! The fixed layouts and flags around files: `open`'s flags, the record `getdents` fills in, and the
//! FAT attribute bits as they appear in that record and in `chmod`'s masks.

// --- open() flags ---------------------------------------------------------------------------

/// Read-only flag for the `flags` argument of the `SYS_OPEN` syscall. Also opens a directory, for
/// `getdents`.
pub const O_RDONLY: usize = 0;
/// Write-only flag for the `flags` argument of the `SYS_OPEN` syscall. Creates the file if it
/// doesn't exist, and always starts it empty.
pub const O_WRONLY: usize = 1;
/// Combined with `O_WRONLY` (`O_WRONLY | O_APPEND`): creates the file if it doesn't exist, but
/// starts writing at its current end instead of emptying it first. Linux's value, so it stays clear
/// of `O_RDONLY`/`O_WRONLY`.
pub const O_APPEND: usize = 0o2000;

// --- Directory records (getdents) --------------------------------------------------------------

/// Longest name a directory record can carry (FAT's long-filename limit).
pub const NAME_MAX: usize = 255;
/// The longest path, in bytes, that `open`, `chmod` and `getcwd` deal with.
pub const PATH_MAX: usize = 4096;
/// Size of one record `getdents` fills in: `size: u32` (little-endian), `attrs: u8`, `name_len: u8`,
/// then `NAME_MAX` bytes of name, NUL-padded.
pub const DIRENT_SIZE: usize = 4 + 1 + 1 + NAME_MAX;

// --- unlink() flags ---------------------------------------------------------------------------

/// Flag for the `flags` argument of the `SYS_UNLINKAT` syscall: the target must be an empty
/// directory, not a file. Linux's value.
pub const AT_REMOVEDIR: usize = 0x200;

// --- stat() payload -----------------------------------------------------------------------------

/// Size of the fixed payload the `stat` syscall writes: `size: u32` (LE), `attrs: u8`,
/// `created_date: u16` (LE), `created_time: u16` (LE), `created_time_tenth: u8`, `modified_date: u16`
/// (LE), `modified_time: u16` (LE), `accessed_date: u16` (LE) -- every field a FAT directory entry
/// actually stores. Dates/times are FAT's own packed encoding (see `hadris_fat::time::FatDateTime`),
/// not calendar values -- unpacking them is left to whichever program displays them.
pub const STAT_SIZE: usize = 16;

// --- FAT attribute bits ------------------------------------------------------------------------

/// Fat attribute: read-only.
pub const ATTR_READ_ONLY: u8 = 0x01;
/// Fat attribute: the volume-label pseudo-entry; not a real file, never listed.
pub const ATTR_VOLUME_LABEL: u8 = 0x08;
/// Fat attribute: directory.
pub const ATTR_DIRECTORY: u8 = 0x10;
/// Fat attribute: executable. Not standard in FAT, uses a reserved bit (i.e. no existing meaning).
pub const ATTR_EXEC: u8 = 0x40;

#[cfg(test)]
mod tests {
    use super::*;

    /// The values r09-r11's own copies (`userlib`, `files.rs`) use.
    #[test]
    fn values_match_the_pre_abi_copies() {
        assert_eq!((O_RDONLY, O_WRONLY), (0, 1));
        assert_eq!(O_APPEND, 0o2000);
        assert_eq!((NAME_MAX, DIRENT_SIZE), (255, 261));
        assert_eq!(
            [ATTR_READ_ONLY, ATTR_VOLUME_LABEL, ATTR_DIRECTORY, ATTR_EXEC],
            [0x01, 0x08, 0x10, 0x40]
        );
    }

    #[test]
    fn step10_values() {
        assert_eq!(AT_REMOVEDIR, 0x200);
        assert_eq!(STAT_SIZE, 16);
    }
}
