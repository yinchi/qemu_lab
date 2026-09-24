//! `tail [-n N] [file]` -- see `docs/progs.md`.
//!
//! A named file is read twice -- once to count its lines, then again to print from the right
//! one -- so it needs no buffer and has no size limit. Stdin can't be reopened (it may be a
//! redirected file or, later, a pipe), so it's buffered whole, up to `STDIN_LIMIT` bytes.

#![no_std]
#![no_main]

use progs::{CHUNK, EINVAL, Input, fail, parse_lines_args, write_all};
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "tail [-n N] [file]";

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
fn emit(chunk: &[u8], skip: &mut usize) -> Result<(), isize> {
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

fn run(args: userlib::Args) -> ExitCode {
    // Usage: tail [-n N] [file]; parser is defined in `lib.rs`.
    let parsed = match parse_lines_args("tail", USAGE, args.skip(1)) {
        Ok(parsed) => parsed,
        Err(status) => return status,
    };

    // Display name for the file or stdin.
    let name = parsed.file.unwrap_or("stdin");

    // Dispatch to the appropriate tail function based on whether a file was specified.
    let result = match parsed.file {
        Some(_) => tail_file(name, parsed.count),
        None => tail_stdin(parsed.count),
    };

    // Handle the result of the tail operation.
    match result {
        Ok(()) => ExitCode(0),
        Err(e) => {
            fail("tail", name, e);
            ExitCode(1)
        }
    }
}

/// Tails the specified file, displaying the last `count` lines.
fn tail_file(name: &str, count: usize) -> Result<(), isize> {

    // Buffer for reading the file in chunks.
    let mut buf = [0u8; CHUNK];

    // Pass 1 of 2: Count the total number of lines in the file.
    let mut counted = LineCount::default();
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
            counted.add(&buf[..n as usize]);
        }
    }

    // Pass 2 of 2: Emit the last `count` lines.
    let mut skip = counted.total().saturating_sub(count);
    let input = Input::open(Some(name))?;
    loop {
        let n = read(input.fd, &mut buf);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            return Ok(());
        }
        emit(&buf[..n as usize], &mut skip)?;
    }
}

/// Tails the standard input, displaying the last `count` lines.
fn tail_stdin(count: usize) -> Result<(), isize> {

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

    // Count the total number of lines in the buffer.
    let mut counted = LineCount::default();
    // Add the final possibly incomplete line to the line count.
    counted.add(&buf[..len]);

    // Determine how many lines to skip before emitting the last `count` lines.
    let mut skip = counted.total().saturating_sub(count);

    // Emit the last `count` lines from the buffer.
    emit(&buf[..len], &mut skip)
}
