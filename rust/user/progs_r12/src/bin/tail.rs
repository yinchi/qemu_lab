//! `tail [-n N | -c N] [file]` -- see `docs/progs.md`. Stage 12's tier replaces the base `tail`
//! (`-n` only) with `-c`.
//!
//! A named file is read twice -- once to count (lines or bytes, depending on mode), then again to
//! print from the right point -- so it needs no buffer and has no size limit. Stdin can't be
//! reopened (it may be a redirected file or, later, a pipe), so it's buffered whole, up to
//! `STDIN_LIMIT` bytes.

#![no_std]
#![no_main]

use progs::{CHUNK, CountMode, EINVAL, Input, diag, parse_lines_or_bytes_args, write_all};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "tail [-n N | -c N] [file]";
const FLAGS: &[(&str, &str)] = &[
    ("-n N", "print the last N lines (default 10)"),
    ("-c N", "print the last N bytes"),
];

/// How much stdin `tail` will buffer. Static rather than on the stack (there's no heap yet), and
/// kept well inside the 2 MiB user window alongside the code and stack.
const STDIN_LIMIT: usize = 512 * 1024;
static mut STDIN_BUF: [u8; STDIN_LIMIT] = [0; STDIN_LIMIT];

/// Counts lines across a stream of chunks: one per newline, plus one for a final line that
/// lacks its newline.
#[derive(Default)]
struct LineCount {
    lines: usize,
    ends_open: bool,
}

impl LineCount {
    /// Reads a chunk of bytes and updates the line count accordingly.
    fn add(&mut self, chunk: &[u8]) {
        let Some(&last) = chunk.last() else { return };
        self.lines += chunk.iter().filter(|&&b| b == b'\n').count();
        self.ends_open = last != b'\n';
    }

    /// Returns the total number of lines counted, including a final line that lacks a newline.
    fn total(&self) -> usize {
        self.lines + self.ends_open as usize
    }
}

/// Writes `chunk` minus its first `*skip` newline-terminated lines, decrementing `*skip` as
/// lines are dropped.
fn emit_lines(chunk: &[u8], skip: &mut usize) -> Result<(), isize> {
    let mut start = 0;

    // Skip the first `*skip` newline-terminated lines in the chunk.
    while *skip > 0 {
        match chunk[start..].iter().position(|&b| b == b'\n') {
            Some(i) => {
                start += i + 1;
                *skip -= 1;
            }

            // No more newlines found in the chunk: the rest of this chunk
            // belongs to a skipped line.
            None => return Ok(()),
        }
    }

    // Write the remaining part of the chunk after skipping the initial lines.
    write_all(1, &chunk[start..])
}

/// Writes `chunk` minus its first `*skip` bytes, decrementing `*skip` as bytes are dropped.
fn emit_bytes(chunk: &[u8], skip: &mut usize) -> Result<(), isize> {
    let start = chunk.len().min(*skip);
    *skip -= start;
    write_all(1, &chunk[start..])
}

fn run(args: userlib::Args) -> ExitCode {
    let parsed = match parse_lines_or_bytes_args("tail", USAGE, FLAGS, args.skip(1)) {
        Ok(parsed) => parsed,
        Err(status) => return status,
    };

    // Display name for the file or stdin.
    let name = parsed.file.unwrap_or("stdin");

    // Dispatch to the appropriate tail function based on whether a file was specified.
    let result = match parsed.file {
        Some(_) => tail_file(name, parsed.mode, parsed.count),
        None => tail_stdin(parsed.mode, parsed.count),
    };

    // Handle the result of the tail operation.
    match result {
        Ok(()) => ExitCode(0),
        Err(e) => {
            diag::input_error("tail", name, e);
            ExitCode(1)
        }
    }
}

/// Tails the specified file, displaying the last `count` lines or bytes, per `mode`.
fn tail_file(name: &str, mode: CountMode, count: usize) -> Result<(), isize> {

    // Buffer for reading the file in chunks.
    let mut buf = [0u8; CHUNK];

    // Pass 1 of 2: measure the file (total lines, or total bytes).
    let mut total_lines = LineCount::default();
    let mut total_bytes = 0usize;
    {
        let input = Input::open(Some(name))?;
        loop {
            let n = read(input.fd, &mut buf);
            if n < 0 {
                return Err(n);
            }
            if n == 0 {
                break;
            }
            let chunk = &buf[..n as usize];
            match mode {
                CountMode::Lines => total_lines.add(chunk),
                CountMode::Bytes => total_bytes += chunk.len(),
            }
        }
    }

    // Pass 2 of 2: emit the last `count` lines or bytes.
    let mut skip = match mode {
        CountMode::Lines => total_lines.total().saturating_sub(count),
        CountMode::Bytes => total_bytes.saturating_sub(count),
    };
    let input = Input::open(Some(name))?;
    loop {
        let n = read(input.fd, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(());
        }
        let chunk = &buf[..n as usize];
        match mode {
            CountMode::Lines => emit_lines(chunk, &mut skip)?,
            CountMode::Bytes => emit_bytes(chunk, &mut skip)?,
        }
    }
}

/// Tails the standard input, displaying the last `count` lines or bytes, per `mode`.
fn tail_stdin(mode: CountMode, count: usize) -> Result<(), isize> {

    // A much larger buffer than for reading files in chunks, since we need to store *all* of stdin
    // up to the limit.
    //
    // SAFETY: this program is single-threaded and this is the only reference to STDIN_BUF.
    #[allow(clippy::deref_addrof)]
    let buf = unsafe { &mut *(&raw mut STDIN_BUF) };

    // Read all of stdin into the buffer, up to the limit.
    let mut len = 0;
    loop {
        if len == buf.len() {
            return Err(EINVAL); // over STDIN_LIMIT
        }

        // Attempt to read the next chunk of stdin into the buffer.
        let n = read(0, &mut buf[len..]);

        // Negative return value indicates an error.
        if n < 0 {
            return Err(n);
        }

        // Zero return value indicates end of input.
        if n == 0 {
            break;
        }

        // Increment the length of the buffer by the number of bytes read.
        len += n as usize;
    }

    let mut skip = match mode {
        CountMode::Lines => {
            let mut counted = LineCount::default();
            counted.add(&buf[..len]);
            counted.total().saturating_sub(count)
        }
        CountMode::Bytes => len.saturating_sub(count),
    };

    // Emit the last `count` lines or bytes from the buffer.
    match mode {
        CountMode::Lines => emit_lines(&buf[..len], &mut skip),
        CountMode::Bytes => emit_bytes(&buf[..len], &mut skip),
    }
}
