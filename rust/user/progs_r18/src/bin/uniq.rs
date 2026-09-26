//! `uniq [-c] [-d] [-u] [file]` -- see `docs/progs.md`. Collapses each run of identical adjacent lines to one (so it wants
//! sorted input, as `sort | uniq` gives it): `-c` prefixes each line with how many there were, `-d` prints only lines that
//! were repeated, `-u` only lines that were not. Standard input for no file, or `-`. Output goes to standard output only.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use getargs::Arg;
use progs::{Fd, Input, diag, fail, help};
use progs_r12::cli;
use progs_r18::read_all;
use progs_r18::textutil::split_lines;
use userlib::{ExitCode, write_stdout};

userlib::entry_with_args!(run);

const USAGE: &str = "uniq [-c] [-d] [-u] [file]";
const FLAGS: &[(&str, &str)] = &[
    ("-c", "prefix each line with the number of times it occurs"),
    ("-d", "print only lines that are repeated"),
    ("-u", "print only lines that are not repeated"),
];

fn run(args: userlib::Args) -> ExitCode {
    cli::status(uniq(args))
}

fn uniq(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let (mut count, mut only_repeated, mut only_unique) = (false, false, false);
    let mut file: Option<&'static str> = None;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("uniq", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('c') | Arg::Long("count") => count = true,
            Arg::Short('d') | Arg::Long("repeated") => only_repeated = true,
            Arg::Short('u') | Arg::Long("unique") => only_unique = true,
            Arg::Positional(name) => {
                if file.replace(name).is_some() {
                    return Err(diag::extra_operand("uniq", name));
                }
            }
            other => return Err(cli::invalid("uniq", other)),
        }
    }

    let name = file.filter(|&n| n != "-");
    let label = file.unwrap_or("-");
    let input = match Input::open(name) {
        Ok(input) => input,
        Err(e) => {
            fail("uniq", label, e);
            return Ok(ExitCode(1));
        }
    };
    let data = match read_all(input.fd) {
        Ok(data) => data,
        Err(e) => {
            fail("uniq", label, e);
            return Ok(ExitCode(1));
        }
    };
    let lines = split_lines(&data);

    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j] == lines[i] {
            j += 1;
        }
        let n = j - i;
        if (!only_repeated || n > 1) && (!only_unique || n == 1) {
            if count {
                let _ = write!(Fd(1), "{n:>7} ");
            }
            write_stdout(lines[i]);
            write_stdout(b"\n");
        }
        i = j;
    }
    Ok(ExitCode(0))
}
