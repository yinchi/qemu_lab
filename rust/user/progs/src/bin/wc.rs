//! `wc [-l] [-w] [-c] [file]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{CHUNK, Fd, Input, fail, unknown_option, usage};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn run(args: userlib::Args) -> ExitCode {

    // Define flags for which counts to display: lines, words, and bytes.
    let (mut lines, mut words, mut bytes) = (false, false, false);

    // Variable to store the name of the file to process, if any.
    let mut file = None;

    for arg in args.skip(1) {
        // Process each argument
        if arg.len() > 1 && arg.starts_with('-') {
            // Process each flag character in the argument (flags can be concatenated, e.g. -lwc).
            for flag in arg[1..].chars() {
                match flag {
                    'l' => lines = true,
                    'w' => words = true,
                    'c' => bytes = true,
                    _ => return unknown_option("wc", arg),
                }
            }
        } else if file.replace(arg).is_some() {
            // Only one file argument is allowed; if another is provided, display usage.
            return usage("wc [-l] [-w] [-c] [file]");
        }
    }

    // If no specific counts were requested, default to counting all three: lines, words, and bytes.
    if !(lines || words || bytes) {
        (lines, words, bytes) = (true, true, true);
    }

    // Display name for the file or stdin.
    let name = file.unwrap_or("stdin");

    // Open the input file or stdin for reading.
    let input = match Input::open(file) {
        Ok(input) => input,
        Err(e) => {
            fail("wc", name, e);
            return ExitCode(1);
        }
    };

    // Initialize counters for lines, words, and bytes.
    let (mut n_lines, mut n_words, mut n_bytes) = (0usize, 0usize, 0usize);

    // Track whether the current position is inside a word.
    let mut in_word = false;

    // Buffer for reading the input in chunks.
    let mut buf = [0u8; CHUNK];

    // Read the input in chunks and update the counters accordingly.
    loop {
        let n = read(input.fd, &mut buf);

        // Negative n indicates an error.
        if n < 0 {
            fail("wc", name, n);
            return ExitCode(1);
        }

        // Zero n indicates end of input.
        if n == 0 {
            break;
        }

        // Update the byte count and analyze each byte to update line and word counts.
        n_bytes += n as usize;
        for &b in &buf[..n as usize] {
            if b == b'\n' {
                n_lines += 1;
            }
            let space = is_space(b);
            if in_word && space {
                in_word = false;
            } else if !in_word && !space {
                in_word = true;
                n_words += 1;
            }
        }
    }

    // Counts are separated by single spaces, in POSIX's fixed order (lines, words, bytes),
    // followed by the file's name if one was given.
    let mut out = Fd(1);
    let mut first = true;

    // Print the counts for the requested categories (lines, words, bytes in order),
    // skipping any that were not requested.
    for (wanted, count) in [(lines, n_lines), (words, n_words), (bytes, n_bytes)] {
        if wanted {
            let _ = write!(out, "{}{count}", if first { "" } else { " " });
            first = false;
        }
    }

    // Print the filename if one was provided (none for stdout).
    if let Some(file) = file {
        let _ = write!(out, " {file}");
    }
    // Ensure the output ends with a newline.
    let _ = writeln!(out);

    ExitCode(0)
}
