//! `tail [-n N | -c N] [file]` -- see `docs/progs.md`. Stage 12's tier replaces the base `tail`
//! (`-n` only) with `-c`.
//!
//! A named file is read twice -- once to count (lines or bytes, depending on mode), then again to
//! print from the right point -- so it needs no buffer and has no size limit. Stdin can't be
//! reopened (it may be a redirected file or a pipe), so it is read once into a growable buffer that only keeps
//! what `tail` could still print: the last N bytes, or the last N lines, however much input goes by. (Stages 12-15,
//! with no heap, buffered stdin whole in a fixed 512 KiB and refused more.)

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use progs::{CHUNK, CountMode, Input, diag, write_all};
use userlib::{ExitCode, read};
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "tail [-n N | -c N] [file]";
const FLAGS: &[(&str, &str)] = &[
    ("-n N", "print the last N lines (default 10)"),
    ("-c N", "print the last N bytes"),
];

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
    let parsed = match cli::count_args("tail", USAGE, FLAGS, args) {
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

/// The most stdin `tail` lets pile up before it trims what could no longer be printed. The buffer is trimmed again
/// each time it doubles, so the copying is amortized: a long stream costs a few passes over what it keeps, not one per chunk.
const TRIM_AT: usize = 64 * 1024;

/// Drops from the front of `window` whatever `tail` could no longer print: everything but the last `count` bytes,
/// or everything before the last `count` lines.
fn trim(window: &mut Vec<u8>, mode: CountMode, count: usize) {
    let drop = match mode {
        CountMode::Bytes => window.len().saturating_sub(count),
        CountMode::Lines => {
            let mut counted = LineCount::default();
            counted.add(window);
            let mut skip = counted.total().saturating_sub(count);
            // The offset just past the `skip`th newline: where the first line worth keeping starts.
            let mut start = 0;
            while skip > 0 {
                match window[start..].iter().position(|&b| b == b'\n') {
                    Some(i) => start += i + 1,
                    None => break,
                }
                skip -= 1;
            }
            start
        }
    };
    window.drain(..drop);
}

/// Tails the standard input, displaying the last `count` lines or bytes, per `mode`.
fn tail_stdin(mode: CountMode, count: usize) -> Result<(), isize> {
    let mut window: Vec<u8> = Vec::new();
    let mut chunk = [0u8; CHUNK];
    let mut trim_at = TRIM_AT;
    loop {
        let n = read(0, &mut chunk);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            break; // end of input
        }
        if window.try_reserve(n as usize).is_err() {
            return Err(abi::errno::ENOMEM);
        }
        window.extend_from_slice(&chunk[..n as usize]);
        if window.len() >= trim_at {
            trim(&mut window, mode, count);
            trim_at = (window.len() * 2).max(TRIM_AT);
        }
    }
    trim(&mut window, mode, count);
    write_all(1, &window)
}
