//! `cat [-n] [-E] [-T] [-s] [file...]` -- see `docs/progs.md`. Stage 18's tier adds to the Stage 12 `cat`: `-n` (number every output
//! line), `-E` (a `$` at the end of each line), `-T` (show a tab as `^I`) and `-s` (squeeze runs of blank lines into one).
//! Numbers and the blank-line memory carry across files, as in GNU's. With none of the flags it is the plain byte copy it always
//! was, so binary files are still safe to `cat`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{CHUNK, copy, fail, help, write_all};
use progs_r12::cli;
use userlib::{ExitCode, O_RDONLY, close, open, read};

userlib::entry_with_args!(run);

const USAGE: &str = "cat [-n] [-E] [-T] [-s] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-n", "number all output lines"),
    ("-E", "display $ at the end of each line"),
    ("-T", "display tabs as ^I"),
    ("-s", "squeeze runs of blank lines into one"),
];

#[derive(Default, Clone, Copy)]
struct Flags {
    number: bool,
    dollar: bool,
    tabs: bool,
    squeeze: bool,
}

impl Flags {
    fn any(self) -> bool {
        self.number || self.dollar || self.tabs || self.squeeze
    }
}

/// Where the output has got to, across chunks and across files.
#[derive(Default)]
struct State {
    /// The number of the last line numbered.
    line_no: usize,
    /// Whether the line being written has started (its number, if any, is out).
    in_line: bool,
    /// Whether the last line written was blank, for `-s`.
    prev_blank: bool,
}

impl State {
    fn number(&mut self, out: &mut Vec<u8>) {
        self.line_no += 1;
        out.extend_from_slice(format!("{:>6}\t", self.line_no).as_bytes());
    }

    /// Appends `chunk`, as the flags say it is shown, to `out`.
    fn show(&mut self, flags: Flags, chunk: &[u8], out: &mut Vec<u8>) {
        for &b in chunk {
            if b == b'\n' {
                if !self.in_line {
                    // A blank line: dropped, if it repeats one and `-s` says so.
                    if flags.squeeze && self.prev_blank {
                        continue;
                    }
                    if flags.number {
                        self.number(out);
                    }
                    self.prev_blank = true;
                } else {
                    self.prev_blank = false;
                }
                if flags.dollar {
                    out.push(b'$');
                }
                out.push(b'\n');
                self.in_line = false;
            } else {
                if !self.in_line {
                    if flags.number {
                        self.number(out);
                    }
                    self.in_line = true;
                }
                if b == b'\t' && flags.tabs {
                    out.extend_from_slice(b"^I");
                } else {
                    out.push(b);
                }
            }
        }
    }
}

/// Shows everything readable from `fd`. `Err` is the negative error from `read` or `write`.
fn show_fd(fd: usize, flags: Flags, state: &mut State) -> Result<(), isize> {
    if !flags.any() {
        return copy(fd, 1);
    }
    let mut buf = [0u8; CHUNK];
    let mut out = Vec::with_capacity(CHUNK * 2);
    loop {
        let n = read(fd, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(());
        }
        out.clear();
        state.show(flags, &buf[..n as usize], &mut out);
        write_all(1, &out)?;
    }
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(concatenate(args))
}

fn concatenate(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut status = 0;
    let mut any_file = false;
    let mut flags = Flags::default();

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cat", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('n') | Arg::Long("number") => flags.number = true,
            Arg::Short('E') | Arg::Long("show-ends") => flags.dollar = true,
            Arg::Short('T') | Arg::Long("show-tabs") => flags.tabs = true,
            Arg::Short('s') | Arg::Long("squeeze-blank") => flags.squeeze = true,
            Arg::Positional(_) => any_file = true,
            other => return Err(cli::invalid("cat", other)),
        }
    }

    let mut state = State::default();
    for path in cli::operands(args) {
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            fail("cat", path, fd);
            status = 1;
            continue;
        }
        if let Err(e) = show_fd(fd as usize, flags, &mut state) {
            fail("cat", path, e);
            status = 1;
        }
        close(fd as usize);
    }

    // If no file arguments were provided, read from stdin.
    if !any_file && let Err(e) = show_fd(0, flags, &mut state) {
        fail("cat", "stdin", e);
        status = 1;
    }

    Ok(ExitCode(status))
}
