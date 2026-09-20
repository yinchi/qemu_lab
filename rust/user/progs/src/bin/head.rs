//! `head [-n N] [file]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{CHUNK, Input, fail, parse_lines_args, write_all};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "head [-n N] [file]";

fn run(args: userlib::Args) -> ExitCode {

    // Get the parsed command-line arguments: line count and input file.
    let parsed = match parse_lines_args("head", USAGE, args.skip(1)) {
        Ok(parsed) => parsed,
        Err(status) => return status,
    };

    // Display name for the file in error messages.
    let name = parsed.file.unwrap_or("stdin");

    // Open the input file (or stdin) for reading.
    let input = match Input::open(parsed.file) {
        Ok(input) => input,
        Err(e) => {
            fail("head", name, e);
            return ExitCode(1);
        }
    };

    // Initialize the remaining line count and the buffer for reading chunks.
    let mut remaining = parsed.count;
    let mut buf = [0u8; CHUNK];

    // Read chunks from the input file and write the first N lines to stdout.
    while remaining > 0 {
        let n = read(input.fd, &mut buf);

        // Negative n: indicates an error occurred while reading.
        if n < 0 {
            fail("head", name, n);
            return ExitCode(1);
        }
        // Zero n: indicates end of file.
        if n == 0 {
            break;
        }
        // Slice the buffer to the number of bytes actually read.
        let chunk = &buf[..n as usize];

        // Emit up to and including the newline that ends the last wanted line.
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

        // Write the chunk up to the determined end position.
        // Return an error if writing fails.
        if let Err(e) = write_all(1, &chunk[..end]) {
            fail("head", "stdout", e);
            return ExitCode(1);
        }
    }
    ExitCode(0)
}
