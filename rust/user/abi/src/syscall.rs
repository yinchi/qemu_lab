//! Syscall numbers -- Linux's real aarch64 values (`include/uapi/asm-generic/unistd.h`), borrowed
//! for familiarity only. The convention itself is the standard AArch64/Linux one: the number in
//! `x8`, up to six arguments in `x0`-`x5`, the return value in `x0`.
//!
//! `SYS_EXIT` is plain `exit` (93), the single-thread-exit syscall, not `exit_group` (94, what
//! glibc's `exit()` calls to tear down every thread) -- irrelevant here since this project has no
//! threads, but named so the choice reads as deliberate.

/// Reserved, not implemented: `chdir`. Its number is held here so a later stage adding it (once the
/// working directory is per-process state -- see `Stage12.md`) doesn't have to pick one.
pub const SYS_CHDIR: usize = 49;
/// `getcwd(buf, len)`: copies the working directory's absolute path (no terminating NUL) into `buf` and
/// returns its length, or `ERANGE` if `buf` is too small. (Not Linux's convention, which also writes a
/// NUL and returns the length including it; the number is Linux's.)
pub const SYS_GETCWD: usize = 17;
pub const SYS_CHMOD: usize = 53;
pub const SYS_OPEN: usize = 56;
pub const SYS_CLOSE: usize = 57;
pub const SYS_GETDENTS: usize = 61;  // Get directory entries
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;

/// `ioctl(fd, request, arg)`: out-of-band control of whatever `fd` is open on -- the request codes
/// are in `abi::ioctl`. Linux's number and Linux's shape; the requests are this project's own.
pub const SYS_IOCTL: usize = 29;

/// `mkdir(path, path_len)`: creates an empty directory. Linux's number for `mkdirat`, shape
/// simplified like every other path-taking syscall here (no dirfd -- paths resolve against the
/// shell's cwd, the same as `open`/`chmod`).
pub const SYS_MKDIRAT: usize = 34;
/// `unlink(path, path_len, flags)`: removes a file, or (`flags & abi::fs::AT_REMOVEDIR`) an empty
/// directory. Linux's number for `unlinkat`, shape simplified as above.
pub const SYS_UNLINKAT: usize = 35;
/// `rename(old, old_len, new, new_len)`: renames or moves an entry within the volume; refuses if
/// `new` already names something. Linux's number for `renameat`, shape simplified as above.
pub const SYS_RENAMEAT: usize = 38;
/// `stat(path, path_len, out)`: writes a path's size, attributes, and timestamps (`abi::fs::STAT_SIZE`
/// bytes) to `out`. Linux's number for `newfstatat` (aarch64 has no plain `stat`), shape simplified
/// as above -- no dirfd, no flags.
pub const SYS_NEWFSTATAT: usize = 79;

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbers r09-r11's own copies use (`syscall.rs`, `userlib`) -- a change that breaks these
    /// breaks every earlier stage's programs.
    #[test]
    fn values_match_the_pre_abi_copies() {
        assert_eq!(
            [SYS_CHMOD, SYS_OPEN, SYS_CLOSE, SYS_GETDENTS, SYS_READ, SYS_WRITE, SYS_EXIT],
            [53, 56, 57, 61, 63, 64, 93]
        );
    }

    #[test]
    fn reserved_numbers_are_linux_numbers() {
        assert_eq!(SYS_CHDIR, 49);
    }

    #[test]
    fn ioctl_is_linuxs_number() {
        assert_eq!(SYS_IOCTL, 29);
    }

    #[test]
    fn getcwd_is_linuxs_number() {
        assert_eq!(SYS_GETCWD, 17);
    }

    #[test]
    fn step10_numbers_are_linuxs_numbers() {
        assert_eq!(
            [SYS_MKDIRAT, SYS_UNLINKAT, SYS_RENAMEAT, SYS_NEWFSTATAT],
            [34, 35, 38, 79]
        );
    }
}
