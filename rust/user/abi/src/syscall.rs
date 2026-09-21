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
pub const SYS_CHMOD: usize = 53;
pub const SYS_OPEN: usize = 56;
pub const SYS_CLOSE: usize = 57;
pub const SYS_GETDENTS: usize = 61;  // Get directory entries
pub const SYS_READ: usize = 63;
pub const SYS_WRITE: usize = 64;
pub const SYS_EXIT: usize = 93;

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
}
