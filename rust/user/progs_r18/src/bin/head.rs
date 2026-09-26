//! `head [-q] [-v] [-n N | -c N] [file...]` -- see `docs/progs.md`. Stage 18's tier extends the Stage 12 `head` with several
//! files (each under a `==> name <==` header, `-q` for none, `-v` for one even with a single file) and a **negative count**:
//! `head -n -5` is everything but the last five lines, `head -c -5` everything but the last five bytes. The other forms print
//! the first N lines or bytes and stop reading. A negative count needs to know where the input ends, so it holds the whole
//! input in the user heap.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{CHUNK, CountMode, Fd, Input, diag, help, write_all};
use progs_r12::cli;
use progs_r18::countspec::{Sign, parse};
use progs_r18::read_all;
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "head [-q] [-v] [-n N | -c N] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n N", "print the first N lines (default 10); -n -N prints all but the last N"),
    ("-c N", "print the first N bytes; -c -N prints all but the last N"),
    ("-q", "never print headers giving file names"),
    ("-v", "always print headers giving file names"),
];

/// How many bytes of `data` come before the last `n` lines: everything if it has no more than `n` lines.
fn all_but_last_lines(data: &[u8], n: usize) -> usize {
    let newlines = data.iter().filter(|&&b| b == b'\n').count();
    let lines = newlines + usize::from(data.last().is_some_and(|&b| b != b'\n'));
    let keep = lines.saturating_sub(n);
    if keep == 0 {
        return 0;
    }
    // The end of the `keep`th line: just past its newline, or the end of the data for an unterminated last line.
    data.iter()
        .enumerate()
        .filter(|&(_, &b)| b == b'\n')
        .nth(keep - 1)
        .map_or(data.len(), |(i, _)| i + 1)
}

/// Prints the leading part of what `fd` gives, as `mode` and `(sign, count)` say. `Err` carries what to report.
fn head_fd(fd: usize, name: &str, mode: CountMode, sign: Sign, count: usize) -> Result<(), ()> {
    if sign == Sign::Minus {
        let data = match read_all(fd) {
            Ok(data) => data,
            Err(e) => {
                diag::report("head", "error reading", name, e);
                return Err(());
            }
        };
        let end = match mode {
            CountMode::Bytes => data.len().saturating_sub(count),
            CountMode::Lines => all_but_last_lines(&data, count),
        };
        if let Err(e) = write_all(1, &data[..end]) {
            diag::write_error("head", e);
            return Err(());
        }
        return Ok(());
    }

    let mut remaining = count;
    let mut buf = [0u8; CHUNK];
    while remaining > 0 {
        let n = read(fd, &mut buf);
        if n < 0 {
            diag::report("head", "error reading", name, n);
            return Err(());
        }
        if n == 0 {
            break;
        }
        let chunk = &buf[..n as usize];
        // Up to the byte count, or up to and including the newline that ends the last wanted line.
        let end = match mode {
            CountMode::Bytes => {
                let end = chunk.len().min(remaining);
                remaining -= end;
                end
            }
            CountMode::Lines => {
                let mut end = chunk.len();
                for (i, &b) in chunk.iter().enumerate() {
                    if b == b'\n' {
                        remaining -= 1;
                        if remaining == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                }
                end
            }
        };
        if let Err(e) = write_all(1, &chunk[..end]) {
            diag::write_error("head", e);
            return Err(());
        }
    }
    Ok(())
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(head(args))
}

fn head(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut mode = None;
    let (mut sign, mut count) = (Sign::None, 10usize);
    let (mut quiet, mut verbose) = (false, false);
    // Operands are gathered here: `cli::operands` would take an option's value (`-n 5`) for one.
    let mut files: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("head", &mut opts)? {
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
            other => return Err(cli::invalid("head", other)),
        };
        if mode.replace(which).is_some() {
            let _ = writeln!(Fd(2), "head: options '-n' and '-c' are mutually exclusive");
            diag::try_help("head");
            return Err(ExitCode(1));
        }
        let text = cli::value("head", &mut opts)?;
        let Some((s, n)) = parse(text) else {
            let _ = writeln!(Fd(2), "head: invalid number of {noun}: '{text}'");
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
        let input = match Input::open(target) {
            Ok(input) => input,
            Err(e) => {
                diag::input_error("head", name, e);
                status = 1;
                continue;
            }
        };
        if show_names {
            let _ = writeln!(Fd(1), "{}==> {name} <==", if printed { "\n" } else { "" });
        }
        printed = true;
        if head_fd(input.fd, name, mode, sign, count).is_err() {
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
