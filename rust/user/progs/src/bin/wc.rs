//! `wc [-l] [-w] [-c] [-L] [file...]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{CHUNK, Fd, fail, help, unknown_option};
use userlib::{ExitCode, O_RDONLY, close, open, read};

userlib::entry_with_args!(run);

const USAGE: &str = "wc [-l] [-w] [-c] [-L] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-l", "count lines"),
    ("-w", "count words"),
    ("-c", "count bytes"),
    ("-L", "report the longest line's length"),
];

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Running counts across one or more chunks of a stream.
#[derive(Default)]
struct Counts {
    lines: usize,
    words: usize,
    bytes: usize,
    max_line: usize,
    cur_line: usize,
    in_word: bool,
}

impl Counts {
    fn add(&mut self, chunk: &[u8]) {
        self.bytes += chunk.len();
        for &b in chunk {
            if b == b'\n' {
                self.lines += 1;
                self.max_line = self.max_line.max(self.cur_line);
                self.cur_line = 0;
            } else {
                self.cur_line += 1;
            }
            let space = is_space(b);
            if self.in_word && space {
                self.in_word = false;
            } else if !self.in_word && !space {
                self.in_word = true;
                self.words += 1;
            }
        }
    }

    /// Accounts for a final line with no trailing newline.
    fn finish(&mut self) {
        self.max_line = self.max_line.max(self.cur_line);
    }

    fn accumulate(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.bytes += other.bytes;
        self.max_line = self.max_line.max(other.max_line);
    }
}

fn count_fd(fd: usize) -> Result<Counts, isize> {
    let mut counts = Counts::default();
    let mut buf = [0u8; CHUNK];
    loop {
        let n = read(fd, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            break;
        }
        counts.add(&buf[..n as usize]);
    }
    counts.finish();
    Ok(counts)
}

fn print_counts(c: &Counts, lines: bool, words: bool, bytes: bool, max_line: bool, name: Option<&str>) {
    let mut out = Fd(1);
    let mut first = true;
    for (wanted, count) in [(lines, c.lines), (words, c.words), (bytes, c.bytes), (max_line, c.max_line)] {
        if wanted {
            let _ = write!(out, "{}{count}", if first { "" } else { " " });
            first = false;
        }
    }
    if let Some(name) = name {
        let _ = write!(out, " {name}");
    }
    let _ = writeln!(out);
}

fn run(args: userlib::Args) -> ExitCode {
    let (mut lines, mut words, mut bytes, mut max_line) = (false, false, false, false);
    let mut n_files = 0usize;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            for flag in arg[1..].chars() {
                match flag {
                    'l' => lines = true,
                    'w' => words = true,
                    'c' => bytes = true,
                    'L' => max_line = true,
                    _ => return unknown_option("wc", arg),
                }
            }
        } else {
            n_files += 1;
        }
    }

    // If nothing at all was requested, default to lines/words/bytes -- matches POSIX's fixed
    // order. `-L` alone is not "nothing": it opts out of the default the same as any other flag.
    if !(lines || words || bytes || max_line) {
        (lines, words, bytes) = (true, true, true);
    }

    let mut status = 0;

    if n_files == 0 {
        match count_fd(0) {
            Ok(c) => print_counts(&c, lines, words, bytes, max_line, None),
            Err(e) => {
                fail("wc", "stdin", e);
                status = 1;
            }
        }
        return ExitCode(status);
    }

    let mut total = Counts::default();
    for arg in args.skip(1) {
        if arg.len() > 1 && arg.starts_with('-') {
            continue; // already validated as a flag above
        }
        let fd = open(arg, O_RDONLY);
        if fd < 0 {
            fail("wc", arg, fd);
            status = 1;
            continue;
        }
        let fd = fd as usize;
        match count_fd(fd) {
            Ok(c) => {
                print_counts(&c, lines, words, bytes, max_line, Some(arg));
                total.accumulate(&c);
            }
            Err(e) => {
                fail("wc", arg, e);
                status = 1;
            }
        }
        close(fd);
    }
    if n_files > 1 {
        print_counts(&total, lines, words, bytes, max_line, Some("total"));
    }

    ExitCode(status)
}
