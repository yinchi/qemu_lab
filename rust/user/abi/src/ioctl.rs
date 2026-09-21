//! `ioctl` request codes: what a program can ask of the thing an fd is open on, out of band -- separate
//! from the bytes it reads and writes, so file contents can never control the display. The codes are
//! this project's own (Linux's are terminal-specific and would only mislead); the error for an fd that
//! does not understand a request is `ENOTTY`, as on Linux.

/// Clears the console and puts its cursor at the top left. Only the console (stdout/stderr, when they
/// are the console) understands it; `arg` is unused.
pub const CONSOLE_CLEAR: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_codes_are_pinned() {
        assert_eq!(CONSOLE_CLEAR, 1);
    }
}
