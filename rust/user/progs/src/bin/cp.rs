//! `cp src dst` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{CHUNK, fail, usage, write_all};
use userlib::{ExitCode, O_RDONLY, O_WRONLY, close, open, read};

userlib::entry_with_args!(run);

fn run(args: userlib::Args) -> ExitCode {
    let mut args = args.skip(1);

    // Expect: `cp src dst`
    // Only one source and one destination argument are expected.
    let (Some(src), Some(dst), None) = (args.next(), args.next(), args.next()) else {
        return usage("cp src dst");
    };

    // Reject any option-like arguments (starting with '-') as unknown options.
    for arg in [src, dst] {
        if arg.len() > 1 && arg.starts_with('-') {
            return progs::unknown_option("cp", arg);
        }
    }

    // Ensure that the source and destination are not the same file.
    if src == dst {
        return usage("cp src dst  (src and dst must differ)");
    }

    // Open the source file for reading.
    let input = open(src, O_RDONLY);
    if input < 0 {
        fail("cp", src, input);
        return ExitCode(1);
    }
    let input = input as usize;

    // Read the first chunk *before* creating (and so truncating) `dst`: a source that can't be
    // read at all -- e.g., a directory -- must not destroy an existing destination first.
    let mut buf = [0u8; CHUNK];
    let first = read(input, &mut buf);
    if first < 0 {
        fail("cp", src, first);
        close(input);
        return ExitCode(1);
    }

    // Open the destination file for writing after successfully reading the first chunk.
    let output = open(dst, O_WRONLY);
    if output < 0 {
        fail("cp", dst, output);
        close(input);
        return ExitCode(1);
    }
    let output = output as usize;

    // Initialize the copy status and start copying the first chunk.
    let mut status = 0;
    let mut n = first;

    // Copy the read chunk, then fetch a new chunk from the source file.
    // Loop to copy chunks until the end of the source file is reached.
    while n > 0 {
        if let Err(e) = write_all(output, &buf[..n as usize]) {
            fail("cp", dst, e);
            status = 1;
            break;
        }

        n = read(input, &mut buf);
        // n == 0 indicates end of file.
        // n < 0 indicates an error occurred while reading.

        if n < 0 {
            fail("cp", src, n);
            status = 1;
            break;
        }
    }
    close(input);

    // Close the output file. The close is what commits the file's size to disk -- a failure
    // here is a failed copy.
    let closed = close(output);
    if closed < 0 {
        fail("cp", dst, closed);
        status = 1;
    }

    ExitCode(status)
}
