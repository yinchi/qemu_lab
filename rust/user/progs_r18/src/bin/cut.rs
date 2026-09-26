//! `cut (-c LIST | -f LIST) [-d C] [-s] [file...]` -- see `docs/progs.md`. Prints selected parts of each line: with `-c` the
//! characters at the positions in LIST (counted in UTF-8 characters, from 1), with `-f` the fields (split at the delimiter `C`, a
//! tab by default) whose numbers are in LIST. A LIST is `N`, `N-M`, `N-`, `-M`, separated by commas. Selected fields are joined
//! with the delimiter, in their original order; a line with no delimiter is printed whole, unless `-s`.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::vec::Vec;
use getargs::Arg;
use progs::{Fd, Input, diag, fail, help};
use progs_r12::cli;
use progs_r18::cutlist::{self, ListError, Range};
use progs_r18::read_all;
use progs_r18::textutil::split_lines;
use userlib::{ExitCode, write_stdout};

userlib::entry_with_args!(run);

const USAGE: &str = "cut (-c LIST | -f LIST) [-d C] [-s] [file...]";
const FLAGS: &[(&str, &str)] = &[
    ("-c LIST", "select these character positions"),
    ("-f LIST", "select these fields"),
    ("-d C", "the field delimiter, one character (default: tab)"),
    ("-s", "with -f, skip lines that have no delimiter"),
];

fn run(args: userlib::Args) -> ExitCode {
    cli::status(cut(args))
}

/// `Err` is the status to return, the problem having been reported.
fn list_error(e: ListError, what: &str, text: &str) -> ExitCode {
    let _ = match e {
        ListError::Invalid => writeln!(Fd(2), "cut: invalid {what} value '{text}'"),
        ListError::Zero => writeln!(Fd(2), "cut: {what}s are numbered from 1"),
        ListError::Decreasing => writeln!(Fd(2), "cut: invalid decreasing range"),
    };
    diag::try_help("cut");
    ExitCode(1)
}

fn cut(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let (mut fields, mut chars): (Option<&'static str>, Option<&'static str>) = (None, None);
    let mut delimiter = "\t";
    let mut only_delimited = false;
    // Operands are gathered here, not by `cli::operands`: that second walk would take an option's value for one.
    let mut names: Vec<&'static str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cut", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('f') | Arg::Long("fields") => fields = Some(cli::value("cut", &mut opts)?),
            Arg::Short('c') | Arg::Long("characters") => chars = Some(cli::value("cut", &mut opts)?),
            Arg::Short('d') | Arg::Long("delimiter") => delimiter = cli::value("cut", &mut opts)?,
            Arg::Short('s') | Arg::Long("only-delimited") => only_delimited = true,
            Arg::Positional(name) => names.push(name),
            other => return Err(cli::invalid("cut", other)),
        }
    }
    if delimiter.chars().count() != 1 {
        let _ = writeln!(Fd(2), "cut: the delimiter must be a single character");
        diag::try_help("cut");
        return Err(ExitCode(1));
    }
    let (ranges, by_field): (Vec<Range>, bool) = match (fields, chars) {
        (None, None) => {
            let _ = writeln!(Fd(2), "cut: you must specify a list of characters or fields");
            diag::try_help("cut");
            return Err(ExitCode(1));
        }
        (Some(_), Some(_)) => {
            let _ = writeln!(Fd(2), "cut: only one type of list may be specified");
            diag::try_help("cut");
            return Err(ExitCode(1));
        }
        (Some(list), None) => (cutlist::parse(list).map_err(|e| list_error(e, "field", list))?, true),
        (None, Some(list)) => (cutlist::parse(list).map_err(|e| list_error(e, "character position", list))?, false),
    };
    if only_delimited && !by_field {
        let _ = writeln!(Fd(2), "cut: suppressing non-delimited lines makes sense only when operating on fields");
        diag::try_help("cut");
        return Err(ExitCode(1));
    }

    if names.is_empty() {
        names.push("-");
    }
    let mut status = 0;
    for name in names {
        let input = match Input::open(if name == "-" { None } else { Some(name) }) {
            Ok(input) => input,
            Err(e) => {
                fail("cut", name, e);
                status = 1;
                continue;
            }
        };
        let data = match read_all(input.fd) {
            Ok(data) => data,
            Err(e) => {
                fail("cut", name, e);
                status = 1;
                continue;
            }
        };
        for line in split_lines(&data) {
            if by_field {
                cut_fields(line, delimiter.as_bytes(), &ranges, only_delimited);
            } else {
                cut_chars(line, &ranges);
            }
        }
    }
    Ok(ExitCode(status))
}

/// The pieces of `line` between occurrences of `delimiter`.
fn split<'a>(line: &'a [u8], delimiter: &[u8]) -> Vec<&'a [u8]> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + delimiter.len() <= line.len() {
        if &line[i..i + delimiter.len()] == delimiter {
            fields.push(&line[start..i]);
            i += delimiter.len();
            start = i;
        } else {
            i += 1;
        }
    }
    fields.push(&line[start..]);
    fields
}

fn cut_fields(line: &[u8], delimiter: &[u8], ranges: &[Range], only_delimited: bool) {
    let fields = split(line, delimiter);
    if fields.len() == 1 {
        // No delimiter in the line: whole, unless it is to be skipped.
        if !only_delimited {
            write_stdout(line);
            write_stdout(b"\n");
        }
        return;
    }
    let mut first = true;
    for (i, field) in fields.iter().enumerate() {
        if cutlist::contains(ranges, i + 1) {
            if !first {
                write_stdout(delimiter);
            }
            write_stdout(field);
            first = false;
        }
    }
    write_stdout(b"\n");
}

/// The characters of `line` at the positions in `ranges`. A line that is not valid UTF-8 is cut by bytes instead.
fn cut_chars(line: &[u8], ranges: &[Range]) {
    match core::str::from_utf8(line) {
        Ok(text) => {
            for (i, (at, c)) in text.char_indices().enumerate() {
                if cutlist::contains(ranges, i + 1) {
                    write_stdout(&line[at..at + c.len_utf8()]);
                }
            }
        }
        Err(_) => {
            for (i, byte) in line.iter().enumerate() {
                if cutlist::contains(ranges, i + 1) {
                    write_stdout(core::slice::from_ref(byte));
                }
            }
        }
    }
    write_stdout(b"\n");
}
