//! `cat [file...]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{copy, fail, help};
use userlib::{ExitCode, O_RDONLY, close, open};
use getargs::Arg;
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "cat [file...]";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    cli::status(concatenate(args))
}

fn concatenate(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut status = 0;
    let mut any_file = false;

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cat", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Positional(_) => any_file = true,
            other => return Err(cli::invalid("cat", other)),
        }
    }

    for path in cli::operands(args) {
        // Open the file for reading.
        let fd = open(path, O_RDONLY);

        // Flag an error if the file could not be opened.
        if fd < 0 {
            fail("cat", path, fd);
            status = 1;
            continue;
        }

        // Copy the contents of the file to stdout.
        // Flag an error if the file could not be copied.
        if let Err(e) = copy(fd as usize, 1) {
            fail("cat", path, e);
            status = 1;
        }

        // Close the file descriptor after copying its contents.
        close(fd as usize);
    }

    // If no file arguments were provided, read from stdin.
    if !any_file && let Err(e) = copy(0, 1) {
        fail("cat", "stdin", e);
        status = 1;
    }

    // Return the exit status; an error on any file or stdin will result in a
    // non-zero exit code.
    Ok(ExitCode(status))
}
