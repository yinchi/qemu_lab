//! Helpers shared by every program in this package -- see `docs/progs.md` for the programs
//! themselves and what each one does and doesn't support.
//!
//! Everything program-level lives here; `userlib` stays the runtime and syscall layer.

#![no_std]

pub mod diag;

use core::fmt;

use userlib::{ExitCode, O_RDONLY, close, open, read, write};

/// Scratch space for the read/write loops below: big enough to make them cheap, small enough to
/// sit comfortably on a program's stack (there's no heap in EL0 until Stage 16).
pub const CHUNK: usize = 4096;

/// A file descriptor as a `core::fmt::Write` sink, so a program can `write!(Fd(1), ...)`
/// instead of hand-assembling numbers into byte buffers.
///
/// `Fd(1)` (stdout) is buffered -- see `userlib::write_stdout` for when it is sent -- and never
/// reports a write error. Every other fd is written immediately, which sends stdout's pending text
/// first, so `Fd(2)` (stderr) output stays in order with stdout even under `2>&1`.
pub struct Fd(pub usize);

impl fmt::Write for Fd {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.0 == 1 {
            userlib::write_stdout(s.as_bytes());
            return Ok(());
        }
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

/// Prints a `--help` synopsis to stdout: `usage: <usage>`, then one `  -x  description` line per
/// entry in `flags`. Every program checks for `--help` before any other argument parsing and
/// returns this directly -- see `docs/progs.md`'s conventions section for why `--help` and not
/// `-h` (POSIX has no help-flag convention at all, and GNU coreutils itself uses the long form
/// only, since `-h` already means something else in some of its utilities).
pub fn help(usage: &str, flags: &[(&str, &str)]) -> ExitCode {
    use core::fmt::Write;
    let mut out = Fd(1);
    let _ = writeln!(out, "usage: {usage}");
    for (flag, desc) in flags {
        let _ = writeln!(out, "  {flag}  {desc}");
    }
    ExitCode(0)
}

/// A path built from `dir` and `name` as `dir/name`, without a heap (there was none in EL0 before Stage 16;
/// `progs_r16::join` returns a `String` and has no limit) -- a fixed `PATH_MAX`-byte buffer instead. Used wherever a program computes a child
/// path itself rather than taking one as an argument (`mv`'s directory-destination case, `rm -r`'s
/// and `chmod -R`'s recursion).
pub struct PathBuf {
    buf: [u8; userlib::PATH_MAX],
    len: usize,
}

impl PathBuf {
    /// Joins `dir` and `name` as `dir/name`, trimming one trailing `/` from `dir` first so joining
    /// under the root (`dir == "/"`) doesn't double it. `None` if the result wouldn't fit in
    /// `PATH_MAX` bytes.
    pub fn join(dir: &str, name: &str) -> Option<Self> {
        let dir = dir.strip_suffix('/').unwrap_or(dir);
        let total = dir.len() + 1 + name.len();
        if total > userlib::PATH_MAX {
            return None;
        }
        let mut buf = [0u8; userlib::PATH_MAX];
        buf[..dir.len()].copy_from_slice(dir.as_bytes());
        buf[dir.len()] = b'/';
        buf[dir.len() + 1..total].copy_from_slice(name.as_bytes());
        Some(Self { buf, len: total })
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: built only from `str` slices above, so the bytes are valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) }
    }
}

/// The text after the last `/` in `path` (or all of it if there is none), after trimming one
/// trailing `/` first so a directory operand's own name is returned rather than an empty string.
/// Used by `mv`'s directory-destination case.
pub fn basename(path: &str) -> &str {
    let path = path.strip_suffix('/').unwrap_or(path);
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
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

/// Whether `head`/`tail`'s count is a number of lines (`-n`, the default) or bytes (`-c`) --
/// Stage 12's extended form. A separate function/type from `parse_lines_args`/`LinesArgs` above
/// (not a modification of them) since those are used by the base tier's `head`/`tail`, which
/// r09-r11 also build and must keep working unchanged.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CountMode {
    Lines,
    Bytes,
}
