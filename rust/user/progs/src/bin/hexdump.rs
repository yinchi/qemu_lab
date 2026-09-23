//! `hexdump [file]` -- output in the format of `hexdump -C`; see `docs/progs.md`.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, Input, fail, help, unknown_option, usage};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

/// Number of bytes per row in the hexdump output.
const WIDTH: usize = 16;

const USAGE: &str = "hexdump [file]";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let mut file = None;

    // Parse command-line arguments to determine the input file.
    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            // Unknown option encountered (we don't support any options).
            return unknown_option("hexdump", arg);
        } else if file.replace(arg).is_some() {
            // More than one file specified; hexdump only supports a single file.
            return usage("hexdump [file]");
        }
    }

    // Display name for the file in error messages.
    let name = file.unwrap_or("stdin");

    // Open the input file (or stdin) for reading.
    let input = match Input::open(file) {
        Ok(input) => input,
        Err(e) => {
            fail("hexdump", name, e);
            return ExitCode(1);
        }
    };

    // Initialize the output file descriptor (stdout), the offset, and the row buffer.
    let mut out = Fd(1);
    let mut offset = 0usize;
    let mut row = [0u8; WIDTH];


    loop {
        // Fill a whole row (a read may return less, notably from stdin, a line at a time).
        let mut filled = 0;

        while filled < WIDTH {
            let n = read(input.fd, &mut row[filled..]);

            // Negative n: indicates an error occurred while reading.
            if n < 0 {
                fail("hexdump", name, n);
                return ExitCode(1);
            }
            // Zero n: indicates end of file.
            if n == 0 {
                break;
            }
            // Positive n: indicates the number of bytes successfully read.
            filled += n as usize;
        }
        if filled == 0 {
            break;
        }

        // Print the offset for the current row.
        let _ = write!(out, "{offset:08x} ");

        // Print the hex representation of each byte in the row.
        for i in 0..WIDTH {

            // Add an extra space in the middle of the row (8 bytes) for readability.
            if i == WIDTH / 2 {
                let _ = write!(out, " ");
            }

            // Print the hex representation of each byte in the row.
            if i < filled {
                let _ = write!(out, " {:02x}", row[i]);
            } else {
                // Print spaces for bytes that were not filled in the last row.
                let _ = write!(out, "   ");
            }
        }

        // Print the ASCII representation of the row.
        let _ = write!(out, "  |");
        for &b in &row[..filled] {
            let c = if (0x20..0x7f).contains(&b) { b as char } else { '.' };
            let _ = write!(out, "{c}");
        }
        let _ = writeln!(out, "|");

        // Update the offset.
        offset += filled;

        // If the last row was not completely filled, we are done.
        if filled < WIDTH {
            break;
        }
    }
    let _ = writeln!(out, "{offset:08x}");
    ExitCode(0)
}
