//! Helpers shared by every program in this package -- see `docs/progs.md` for the programs
//! themselves and what each one does and doesn't support.
//!
//! Everything program-level lives here; `userlib` stays the runtime and syscall layer.

#![no_std]

use core::fmt;

use userlib::{ExitCode, O_RDONLY, close, open, read, write};

/// Scratch space for the read/write loops below: big enough to make them cheap, small enough to
/// sit comfortably on a program's stack (there's no heap in EL0 until Stage 18).
pub const CHUNK: usize = 4096;

/// A file descriptor as a `core::fmt::Write` sink, so a program can `write!(Fd(1), ...)`
/// instead of hand-assembling numbers into byte buffers.
pub struct Fd(pub usize);

impl fmt::Write for Fd {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_all(self.0, s.as_bytes()).map_err(|_| fmt::Error)
    }
}

/// Writes all of `bytes` to `fd`, retrying after a short write. `Err` carries the negative
/// error `write` returned.
pub fn write_all(fd: usize, mut bytes: &[u8]) -> Result<(), isize> {
    while !bytes.is_empty() {
        let n = write(fd, bytes);
        if n < 0 {
            return Err(n);
        }
        bytes = &bytes[n as usize..];
    }
    Ok(())
}

/// `Invalid argument`, for the few errors a program reports on its own rather than getting from a
/// syscall -- same value and same convention as the kernel's (both come from the `abi` crate).
pub use abi::errno::EINVAL;

/// Human-readable text for a negative error a syscall returned -- see `abi::errno::errmsg`.
pub fn errmsg(code: isize) -> &'static str {
    abi::errno::errmsg(code)
}

/// Prints `prog: what: <message for code>` to stderr -- the shape every failing program uses.
pub fn fail(prog: &str, what: &str, code: isize) {
    use core::fmt::Write;
    let _ = writeln!(Fd(2), "{prog}: {what}: {}", errmsg(code));
}

/// Prints `prog: unknown option: arg` to stderr and returns the exit status for it.
pub fn unknown_option(prog: &str, arg: &str) -> ExitCode {
    use core::fmt::Write;
    let _ = writeln!(Fd(2), "{prog}: unknown option: {arg}");
    ExitCode(1)
}

/// Prints `usage: <text>` to stderr and returns the exit status for it.
pub fn usage(text: &str) -> ExitCode {
    use core::fmt::Write;
    let _ = writeln!(Fd(2), "usage: {text}");
    ExitCode(1)
}

/// Parses a non-negative decimal integer -- digits only, no sign, no whitespace, no overflow.
pub fn atoi(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

/// Copies everything readable from `from` to `to`, until end of file. `Err` carries the negative
/// error from whichever side failed first.
pub fn copy(from: usize, to: usize) -> Result<(), isize> {
    let mut buf = [0u8; CHUNK];
    loop {
        let n = read(from, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(());
        }
        write_all(to, &buf[..n as usize])?;
    }
}

/// An open input for a program that takes an optional file: `path` if given, else stdin. Closes
/// the fd on drop, unless it's stdin.
pub struct Input {
    pub fd: usize,
    owned: bool,
}

impl Input {
    /// Opens `path` for reading, or wraps stdin (fd `0`) if `path` is `None`. `Err` carries the
    /// negative error from `open`.
    pub fn open(path: Option<&str>) -> Result<Self, isize> {
        match path {
            None => Ok(Self { fd: 0, owned: false }),
            Some(path) => {
                let fd = open(path, O_RDONLY);
                if fd < 0 {
                    Err(fd)
                } else {
                    Ok(Self { fd: fd as usize, owned: true })
                }
            }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        if self.owned {
            close(self.fd);
        }
    }
}

/// The arguments `head` and `tail` share: `[-n N] [file]`.
pub struct LinesArgs {
    pub count: usize,
    pub file: Option<&'static str>,
}

/// Parses `[-n N] [file]` from `args` (already past `argv[0]`). `Err` is the exit status to
/// return, the problem having been reported already.
pub fn parse_lines_args(
    prog: &str,
    usage_text: &str,
    mut args: impl Iterator<Item = &'static str>,
) -> Result<LinesArgs, ExitCode> {
    let mut parsed = LinesArgs { count: 10, file: None };
    while let Some(arg) = args.next() {
        if arg == "-n" {
            let Some(count) = args.next().and_then(atoi) else {
                return Err(usage(usage_text));
            };
            parsed.count = count;
        } else if arg.len() > 1 && arg.starts_with('-') {
            return Err(unknown_option(prog, arg));
        } else if parsed.file.replace(arg).is_some() {
            return Err(usage(usage_text));
        }
    }
    Ok(parsed)
}
