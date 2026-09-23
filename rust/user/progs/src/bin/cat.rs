//! `cat [file...]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{copy, fail, help, unknown_option};
use userlib::{ExitCode, O_RDONLY, close, open};

userlib::entry_with_args!(run);

const USAGE: &str = "cat [file...]";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let mut status = 0;
    let mut any_file = false;

    for path in args.skip(1) {
        if path == "--help" {
            return help(USAGE, FLAGS);
        }

        // Reject any option-like arguments (starting with '-') as unknown options.
        if path.len() > 1 && path.starts_with('-') {
            return unknown_option("cat", path);
        }

        // Flag that we have encountered at least one file argument.
        any_file = true;

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
    if !any_file {
        if let Err(e) = copy(0, 1) {
            fail("cat", "stdin", e);
            status = 1;
        }
    }

    // Return the exit status; an error on any file or stdin will result in a
    // non-zero exit code.
    ExitCode(status)
}
