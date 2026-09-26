//! `tail [-q] [-v] [-n N | -c N] [file...]` -- see `docs/progs.md`. Stage 18's tier extends the Stage 16 `tail` with several
//! files (each under a `==> name <==` header, `-q` for none, `-v` for one even with a single file) and a **plus count**:
//! `tail -n +5` prints from the fifth line on, `tail -c +5` from the fifth byte. Without a sign the count is the last N lines or
//! bytes, as before.
//!
//! A named file is read twice for the last N -- once to count, then again to print from the right point -- so it needs no buffer
//! and has no size limit. Stdin can't be reopened (it may be a redirected file or a pipe), so it is read once into a growable
//! buffer that only keeps what `tail` could still print. A plus count needs neither: it skips N-1 lines or bytes and copies the rest.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{CHUNK, CountMode, Fd, Input, diag, help, write_all};
use progs_r12::cli;
use progs_r18::countspec::{Sign, parse};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "tail [-q] [-v] [-n N | -c N] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n N", "print the last N lines (default 10); -n +N prints from line N on"),
    ("-c N", "print the last N bytes; -c +N prints from byte N on"),
    ("-q", "never print headers giving file names"),
    ("-v", "always print headers giving file names"),
];

/// Counts lines across a stream of chunks: one per newline, plus one for a final line that lacks its newline.
#[derive(Default)]
struct LineCount {
    lines: usize,
    ends_open: bool,
}

impl LineCount {
    fn add(&mut self, chunk: &[u8]) {
        let Some(&last) = chunk.last() else { return };
        self.lines += chunk.iter().filter(|&&b| b == b'\n').count();
        self.ends_open = last != b'\n';
    }

    fn total(&self) -> usize {
        self.lines + self.ends_open as usize
    }
}

/// Writes `chunk` minus its first `*skip` newline-terminated lines, decrementing `*skip` as lines are dropped.
fn emit_lines(chunk: &[u8], skip: &mut usize) -> Result<(), isize> {
    let mut start = 0;
    while *skip > 0 {
        match chunk[start..].iter().position(|&b| b == b'\n') {
            Some(i) => {
                start += i + 1;
                *skip -= 1;
            }
            // No more newlines in the chunk: the rest of it belongs to a skipped line.
            None => return Ok(()),
        }
    }
    write_all(1, &chunk[start..])
}

/// Writes `chunk` minus its first `*skip` bytes, decrementing `*skip` as bytes are dropped.
fn emit_bytes(chunk: &[u8], skip: &mut usize) -> Result<(), isize> {
    let start = chunk.len().min(*skip);
    *skip -= start;
    write_all(1, &chunk[start..])
}

/// Copies what `fd` gives to stdout, dropping the first `skip` lines or bytes (`tail -n +N`: `skip` is N-1).
fn from_start(fd: usize, mode: CountMode, mut skip: usize) -> Result<(), isize> {
    let mut buf = [0u8; CHUNK];
    loop {
        let n = read(fd, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(());
        }
        let chunk = &buf[..n as usize];
        match mode {
            CountMode::Lines => emit_lines(chunk, &mut skip)?,
            CountMode::Bytes => emit_bytes(chunk, &mut skip)?,
        }
    }
}

/// Tails the file `name` (opened afresh for each of its two passes): the last `count` lines or bytes.
fn last_of_file(name: &str, mode: CountMode, count: usize) -> Result<(), isize> {
    let mut buf = [0u8; CHUNK];
    // Pass 1 of 2: measure the file (total lines, or total bytes).
    let mut total_lines = LineCount::default();
    let mut total_bytes = 0usize;
    {
        let input = Input::open(Some(name))?;
        loop {
            let n = read(input.fd, &mut buf);
            if n < 0 {
                return Err(n);
            }
            if n == 0 {
                break;
            }
            match mode {
                CountMode::Lines => total_lines.add(&buf[..n as usize]),
                CountMode::Bytes => total_bytes += n as usize,
            }
        }
    }
    // Pass 2 of 2: print from the right point.
    let skip = match mode {
        CountMode::Lines => total_lines.total().saturating_sub(count),
        CountMode::Bytes => total_bytes.saturating_sub(count),
    };
    let input = Input::open(Some(name))?;
    from_start(input.fd, mode, skip)
}

/// The most stdin `tail` lets pile up before it trims what could no longer be printed. The buffer is trimmed again each time it doubles,
/// so the copying is amortized.
const TRIM_AT: usize = 64 * 1024;

/// Drops from the front of `window` whatever `tail` could no longer print: everything but the last `count` bytes, or everything before
/// the last `count` lines.
fn trim(window: &mut Vec<u8>, mode: CountMode, count: usize) {
    let drop = match mode {
        CountMode::Bytes => window.len().saturating_sub(count),
        CountMode::Lines => {
            let mut counted = LineCount::default();
            counted.add(window);
            let mut skip = counted.total().saturating_sub(count);
            let mut start = 0;
            while skip > 0 {
                match window[start..].iter().position(|&b| b == b'\n') {
                    Some(i) => start += i + 1,
                    None => break,
                }
                skip -= 1;
            }
            start
        }
    };
    window.drain(..drop);
}

/// Tails the standard input: the last `count` lines or bytes.
fn last_of_stdin(mode: CountMode, count: usize) -> Result<(), isize> {
    let mut window: Vec<u8> = Vec::new();
    let mut chunk = [0u8; CHUNK];
    let mut trim_at = TRIM_AT;
    loop {
        let n = read(0, &mut chunk);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            break;
        }
        if window.try_reserve(n as usize).is_err() {
            return Err(abi::errno::ENOMEM);
        }
        window.extend_from_slice(&chunk[..n as usize]);
        if window.len() >= trim_at {
            trim(&mut window, mode, count);
            trim_at = (window.len() * 2).max(TRIM_AT);
        }
    }
    trim(&mut window, mode, count);
    write_all(1, &window)
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(tail(args))
}

fn tail(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut mode = None;
    let (mut sign, mut count) = (Sign::None, 10usize);
    let (mut quiet, mut verbose) = (false, false);
    // Operands are gathered here: `cli::operands` would take an option's value (`-n 5`) for one.
    let mut files: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("tail", &mut opts)? {
        let (which, noun) = match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('n') | Arg::Long("lines") => (CountMode::Lines, "lines"),
            Arg::Short('c') | Arg::Long("bytes") => (CountMode::Bytes, "bytes"),
            Arg::Short('q') | Arg::Long("quiet" | "silent") => {
                quiet = true;
                continue;
            }
            Arg::Short('v') | Arg::Long("verbose") => {
                verbose = true;
                continue;
            }
            Arg::Positional(operand) => {
                files.push(operand);
                continue;
            }
            other => return Err(cli::invalid("tail", other)),
        };
        if mode.replace(which).is_some() {
            let _ = writeln!(Fd(2), "tail: options '-n' and '-c' are mutually exclusive");
            diag::try_help("tail");
            return Err(ExitCode(1));
        }
        let text = cli::value("tail", &mut opts)?;
        let Some((s, n)) = parse(text) else {
            let _ = writeln!(Fd(2), "tail: invalid number of {noun}: '{text}'");
            return Err(ExitCode(1));
        };
        (sign, count) = (s, n);
    }
    let mode = mode.unwrap_or(CountMode::Lines);

    let show_names = verbose || (files.len() > 1 && !quiet);
    let targets: Vec<Option<&'static str>> = if files.is_empty() { alloc::vec![None] } else { files.iter().map(|&f| Some(f)).collect() };
    let mut status = 0;
    let mut printed = false;
    for target in targets {
        let name = target.unwrap_or("standard input");
        // Open once to see that it can be (and to report as `head` does); the last-N form opens again by name.
        let input = match Input::open(target) {
            Ok(input) => input,
            Err(e) => {
                diag::input_error("tail", name, e);
                status = 1;
                continue;
            }
        };
        if show_names {
            let _ = writeln!(Fd(1), "{}==> {name} <==", if printed { "\n" } else { "" });
        }
        printed = true;
        let result = match (sign, target) {
            // `+N`: from the Nth on, in one pass; `+0` and `+1` both mean everything.
            (Sign::Plus, _) => from_start(input.fd, mode, count.saturating_sub(1)),
            (_, Some(file)) => {
                drop(input);
                last_of_file(file, mode, count)
            }
            (_, None) => last_of_stdin(mode, count),
        };
        if let Err(e) = result {
            diag::input_error("tail", name, e);
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
