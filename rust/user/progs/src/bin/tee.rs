//! `tee [-a] [file...]` -- see `docs/progs.md`.
//!
//! Copies stdin to stdout, also writing the same bytes to every named file -- POSIX allows zero
//! file operands (just an odd way to copy stdin to stdout), so that isn't a usage error here either.
//! Destination files are all opened up front, before anything is read, and held open for the whole
//! copy (no heap in EL0 to hold a growable list, so a fixed-size array instead -- `MAX_FILES` is
//! generous for realistic use, not a hard POSIX limit).

#![no_std]
#![no_main]

use abi::errno::EMFILE;
use progs::{CHUNK, fail, help, unknown_option, write_all};
use userlib::{ExitCode, O_APPEND, O_WRONLY, close, open, read};

userlib::entry_with_args!(run);

const USAGE: &str = "tee [-a] [file...]";
const FLAGS: &[(&str, &str)] = &[("-a", "append to each file instead of truncating it")];

/// How many destination files `tee` can hold open at once.
const MAX_FILES: usize = 8;

fn run(args: userlib::Args) -> ExitCode {
    let mut append = false;
    let mut paths: [&str; MAX_FILES] = [""; MAX_FILES];
    let mut handles: [Option<usize>; MAX_FILES] = [None; MAX_FILES];
    let mut n = 0usize;
    let mut status = 0;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg == "-a" {
            append = true;
            continue;
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("tee", arg);
        }
        if n == MAX_FILES {
            fail("tee", arg, EMFILE);
            status = 1;
            continue;
        }
        let flags = O_WRONLY | if append { O_APPEND } else { 0 };
        let fd = open(arg, flags);
        if fd < 0 {
            fail("tee", arg, fd);
            status = 1;
            continue;
        }
        paths[n] = arg;
        handles[n] = Some(fd as usize);
        n += 1;
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

        for i in 0..n {
            let Some(handle) = handles[i] else { continue };
            if let Err(e) = write_all(handle, chunk) {
                fail("tee", paths[i], e);
                status = 1;
                handles[i] = None; // stop writing to this one; report it only once
            }
        }
    }

    for i in 0..n {
        let Some(handle) = handles[i] else { continue };
        let closed = close(handle);
        if closed < 0 {
            fail("tee", paths[i], closed);
            status = 1;
        }
    }

    ExitCode(status)
}
