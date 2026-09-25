//! `tee [-a] [file...]` -- see `docs/progs.md`.
//!
//! Copies stdin to stdout, also writing the same bytes to every named file -- POSIX allows zero
//! file operands (just an odd way to copy stdin to stdout), so that isn't a usage error here either.
//! Destination files are all opened up front, before anything is read, and held open for the whole
//! copy, in a growable list: there is no limit of `tee`'s own, only the kernel's on open files (a file
//! past it is reported `Too many open files` and skipped). Stages 12-15, with no heap, held them in a
//! fixed array of 8.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use progs::{CHUNK, fail, help, write_all};
use userlib::{ExitCode, O_APPEND, O_WRONLY, close, open, read};
use getargs::Arg;
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "tee [-a] [file...]";
const FLAGS: &[(&str, &str)] = &[("-a", "append to each file instead of truncating it")];

fn run(args: userlib::Args) -> ExitCode {
    let mut append = false;
    // One entry per destination that opened: its name (for messages) and its descriptor, until it fails.
    let mut outputs: Vec<(&str, Option<usize>)> = Vec::new();
    let mut status = 0;

    // First pass: the flags, so that `-a` applies to every file wherever it is written (as in GNU).
    let mut opts = cli::opts(args);
    loop {
        match cli::next("tee", &mut opts) {
            Err(status) => return status,
            Ok(None) => break,
            Ok(Some(Arg::Long("help"))) => return help(USAGE, FLAGS),
            Ok(Some(Arg::Short('a') | Arg::Long("append"))) => append = true,
            Ok(Some(Arg::Positional(_))) => {}
            Ok(Some(other)) => return cli::invalid("tee", other),
        }
    }

    for arg in cli::operands(args) {
        let flags = O_WRONLY | if append { O_APPEND } else { 0 };
        let fd = open(arg, flags);
        if fd < 0 {
            fail("tee", arg, fd);
            status = 1;
            continue;
        }
        outputs.push((arg, Some(fd as usize)));
    }

    let mut buf = [0u8; CHUNK];
    loop {
        let read_n = read(0, &mut buf);
        if read_n < 0 {
            fail("tee", "stdin", read_n);
            status = 1;
            break;
        }
        if read_n == 0 {
            break;
        }
        let chunk = &buf[..read_n as usize];

        if let Err(e) = write_all(1, chunk) {
            fail("tee", "stdout", e);
            status = 1;
        }

        for (path, handle) in outputs.iter_mut() {
            let Some(fd) = *handle else { continue };
            if let Err(e) = write_all(fd, chunk) {
                fail("tee", path, e);
                status = 1;
                *handle = None; // stop writing to this one; report it only once
            }
        }
    }

    for (path, handle) in &outputs {
        let Some(fd) = *handle else { continue };
        let closed = close(fd);
        if closed < 0 {
            fail("tee", path, closed);
            status = 1;
        }
    }

    ExitCode(status)
}
