//! `head [-n N | -c N] [file]` -- see `docs/progs.md`. Stage 12's tier replaces the base `head`
//! (`-n` only) with `-c`.

#![no_std]
#![no_main]

use progs::{CHUNK, CountMode, Input, diag, write_all};
use userlib::{ExitCode, read};
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "head [-n N | -c N] [file]";
const FLAGS: &[(&str, &str)] = &[
    ("-n N", "print the first N lines (default 10)"),
    ("-c N", "print the first N bytes"),
];

fn run(args: userlib::Args) -> ExitCode {

    // Get the parsed command-line arguments: line/byte count and input file.
    let parsed = match cli::count_args("head", USAGE, FLAGS, args) {
        Ok(parsed) => parsed,
        Err(status) => return status,
    };

    // Display name for the file in error messages.
    let name = parsed.file.unwrap_or("stdin");

    // Open the input file (or stdin) for reading.
    let input = match Input::open(parsed.file) {
        Ok(input) => input,
        Err(e) => {
            diag::input_error("head", name, e);
            return ExitCode(1);
        }
    };

    let mut remaining = parsed.count;
    let mut buf = [0u8; CHUNK];

    // Read chunks from the input file and write the first N lines or bytes to stdout.
    while remaining > 0 {
        let n = read(input.fd, &mut buf);

        // Negative n: indicates an error occurred while reading.
        if n < 0 {
            diag::report("head", "error reading", name, n);
            return ExitCode(1);
        }
        // Zero n: indicates end of file.
        if n == 0 {
            break;
        }
        // Slice the buffer to the number of bytes actually read.
        let chunk = &buf[..n as usize];

        // Emit up to the determined end position: the byte count, in byte mode, or up to and
        // including the newline that ends the last wanted line, in line mode.
        let end = match parsed.mode {
            CountMode::Bytes => {
                let end = chunk.len().min(remaining);
                remaining -= end;
                end
            }
            CountMode::Lines => {
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
                end
            }
        };

        // Write the chunk up to the determined end position.
        // Return an error if writing fails.
        if let Err(e) = write_all(1, &chunk[..end]) {
            diag::write_error("head", e);
            return ExitCode(1);
        }
    }
    ExitCode(0)
}
