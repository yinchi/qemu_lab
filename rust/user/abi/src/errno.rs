//! Error return values: a negated Linux errno number in `x0`, the same convention Linux's own
//! syscall ABI uses -- any negative `isize` is an error and the magnitude says which. `errmsg` turns
//! one into text, in the wording of `strerror`.

/// No such file or directory.
pub const ENOENT: isize = -2;
/// I/O error.
pub const EIO: isize = -5;
/// Argument list too long.
pub const E2BIG: isize = -7;
/// Exec format error.
pub const ENOEXEC: isize = -8;
/// Bad file descriptor.
pub const EBADF: isize = -9;
/// Permission denied.
pub const EACCES: isize = -13;
/// Bad address.
pub const EFAULT: isize = -14;
/// File exists.
pub const EEXIST: isize = -17;
/// Not a directory.
pub const ENOTDIR: isize = -20;
/// Is a directory.
pub const EISDIR: isize = -21;
/// Invalid argument.
pub const EINVAL: isize = -22;
/// Too many open files.
pub const EMFILE: isize = -24;
/// Inappropriate ioctl for device -- here: the fd is not the console.
pub const ENOTTY: isize = -25;
/// No space left on device.
pub const ENOSPC: isize = -28;
/// Numerical result out of range -- here: the buffer is too small for the result.
pub const ERANGE: isize = -34;
/// File name too long.
pub const ENAMETOOLONG: isize = -36;
/// Function not implemented.
pub const ENOSYS: isize = -38;
/// Directory not empty.
pub const ENOTEMPTY: isize = -39;

/// Human-readable text for a negative error a syscall returned (`strerror`'s wording).
pub fn errmsg(code: isize) -> &'static str {
    match code {
        ENOENT => "No such file or directory",
        EIO => "I/O error",
        E2BIG => "Argument list too long",
        ENOEXEC => "Exec format error",
        EBADF => "Bad file descriptor",
        EACCES => "Permission denied",
        EFAULT => "Bad address",
        EEXIST => "File exists",
        ENOTDIR => "Not a directory",
        EISDIR => "Is a directory",
        EINVAL => "Invalid argument",
        EMFILE => "Too many open files",
        ENOTTY => "Inappropriate ioctl for device",
        ENOSPC => "No space left on device",
        ERANGE => "Numerical result out of range",
        ENAMETOOLONG => "File name too long",
        ENOSYS => "Function not implemented",
        ENOTEMPTY => "Directory not empty",
        _ => "Unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values r09-r11's own `errno.rs` copies use.
    #[test]
    fn values_match_the_pre_abi_copies() {
        assert_eq!(
            [EIO, EBADF, EACCES, ENOENT, ENOTDIR, EISDIR, EINVAL, EMFILE],
            [-5, -9, -13, -2, -20, -21, -22, -24]
        );
    }

    /// Linux's real numbers for the values added since.
    #[test]
    fn new_values_are_linux_numbers() {
        assert_eq!(
            [E2BIG, ENOEXEC, EFAULT, EEXIST, ENOTTY, ENOSPC, ERANGE, ENAMETOOLONG, ENOSYS, ENOTEMPTY],
            [-7, -8, -14, -17, -25, -28, -34, -36, -38, -39]
        );
    }

    #[test]
    fn every_error_has_a_message() {
        for code in [
            ENOENT, EIO, E2BIG, ENOEXEC, EBADF, EACCES, EFAULT, EEXIST, ENOTDIR, EISDIR, EINVAL,
            EMFILE, ENOTTY, ENOSPC, ERANGE, ENAMETOOLONG, ENOSYS, ENOTEMPTY,
        ] {
            assert_ne!(errmsg(code), "Unknown error", "{code}");
        }
        assert_eq!(errmsg(-1000), "Unknown error");
        assert_eq!(errmsg(ENOEXEC), "Exec format error");
    }
}
