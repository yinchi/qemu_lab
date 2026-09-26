//! `cp [-r] [-n] [-v] SRC... DST` -- see `docs/progs.md`. Stage 18's tier adds `-r`/`-R` (copy directories, recursively),
//! `-n` (never overwrite an existing file, silently) and `-v` (say what was copied) to the Stage 16 `cp`.
//!
//! A directory is copied by creating the destination directory and copying its entries into it, in name order; an
//! existing destination directory is merged into. Attributes (the executable and read-only bits) and times are not
//! preserved: there is no syscall to set either.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use abi::errno::{EISDIR, ENOENT};
use alloc::string::String;
use getargs::Arg;
use progs::{CHUNK, Fd, basename, diag, help, write_all};
use progs_r12::cli;
use progs_r18::{join, read_dir};
use userlib::{ATTR_DIRECTORY, ExitCode, O_RDONLY, O_WRONLY, close, mkdir, open, read, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "cp [-r] [-n] [-v] SRC... DST";
const FLAGS: &[(&str, &str)] = &[
    ("-r", "copy directories recursively"),
    ("-n", "do not overwrite an existing file"),
    ("-v", "print what is being copied"),
];

struct Options {
    recursive: bool,
    no_clobber: bool,
    verbose: bool,
}

fn announce(o: &Options, src: &str, dst: &str) {
    if o.verbose {
        let _ = writeln!(Fd(1), "'{src}' -> '{dst}'");
    }
}

/// Copies the file `src` to `dst`, reporting in GNU's wording (attributed to whichever side actually failed) and
/// returning `Err(())` if it didn't work. Reads the first chunk *before* creating (and so truncating) `dst`: a source that
/// can't be read at all must not destroy an existing destination first.
fn copy_file(src: &str, dst: &str, o: &Options) -> Result<(), ()> {
    if o.no_clobber && stat(dst).is_ok() {
        return Ok(());
    }
    let input = open(src, O_RDONLY);
    if input < 0 {
        if input == ENOENT {
            diag::cannot("cp", "stat", src, input);
        } else {
            diag::input_error("cp", src, input);
        }
        return Err(());
    }
    let input = input as usize;

    let mut buf = [0u8; CHUNK];
    let first = read(input, &mut buf);
    if first < 0 {
        if first == EISDIR {
            let _ = writeln!(Fd(2), "cp: -r not specified; omitting directory '{src}'");
        } else {
            diag::report("cp", "error reading", src, first);
        }
        close(input);
        return Err(());
    }

    let output = open(dst, O_WRONLY);
    if output < 0 {
        diag::cannot("cp", "create regular file", dst, output);
        close(input);
        return Err(());
    }
    let output = output as usize;

    let mut status = Ok(());
    let mut n = first;
    while n > 0 {
        if let Err(e) = write_all(output, &buf[..n as usize]) {
            diag::report("cp", "error writing", dst, e);
            status = Err(());
            break;
        }
        n = read(input, &mut buf);
        if n < 0 {
            diag::report("cp", "error reading", src, n);
            status = Err(());
            break;
        }
    }
    close(input);

    // Closing is what commits the file's size to disk -- a failure here is a failed copy.
    let closed = close(output);
    if closed < 0 && status.is_ok() {
        diag::report("cp", "error writing", dst, closed);
        status = Err(());
    }
    if status.is_ok() {
        announce(o, src, dst);
    }
    status
}

/// Copies the directory `src` to `dst` and everything under it. `dst` is created, or merged into if it is already a
/// directory; a failure on one entry is reported and the rest are still copied, and the whole is `Err(())`.
fn copy_tree(src: &str, dst: &str, o: &Options) -> Result<(), ()> {
    match stat(dst) {
        Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {}
        Ok(_) => {
            let _ = writeln!(Fd(2), "cp: cannot overwrite non-directory '{dst}' with directory '{src}'");
            return Err(());
        }
        Err(ENOENT) => {
            let r = mkdir(dst);
            if r < 0 {
                diag::cannot("cp", "create directory", dst, r);
                return Err(());
            }
        }
        Err(e) => {
            diag::cannot("cp", "stat", dst, e);
            return Err(());
        }
    }
    announce(o, src, dst);

    let mut entries = match read_dir(src) {
        Ok(entries) => entries,
        Err(e) => {
            diag::report("cp", "error reading", src, e.errno());
            return Err(());
        }
    };
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut result = Ok(());
    for ent in entries {
        let (from, to) = (join(src, &ent.name), join(dst, &ent.name));
        let copied = if ent.attrs & ATTR_DIRECTORY != 0 { copy_tree(&from, &to, o) } else { copy_file(&from, &to, o) };
        if copied.is_err() {
            result = Err(());
        }
    }
    result
}

/// Copies one operand: a directory with `-r`, a file otherwise.
fn copy_operand(src: &str, target: &str, o: &Options) -> Result<(), ()> {
    match stat(src) {
        Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
            if !o.recursive {
                let _ = writeln!(Fd(2), "cp: -r not specified; omitting directory '{src}'");
                return Err(());
            }
            // A directory cannot be copied into itself.
            let plain = src.trim_end_matches('/');
            if target == plain || target.strip_prefix(plain).is_some_and(|rest| rest.starts_with('/')) {
                let _ = writeln!(Fd(2), "cp: cannot copy a directory, '{src}', into itself, '{target}'");
                return Err(());
            }
            copy_tree(plain, target, o)
        }
        _ => copy_file(src, target, o),
    }
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(copy(args))
}

fn copy(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut o = Options { recursive: false, no_clobber: false, verbose: false };
    let (mut count, mut first, mut last) = (0usize, None, None);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cp", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => o.recursive = true,
            Arg::Short('n') | Arg::Long("no-clobber") => o.no_clobber = true,
            Arg::Short('v') | Arg::Long("verbose") => o.verbose = true,
            Arg::Positional(operand) => {
                count += 1;
                first.get_or_insert(operand);
                last = Some(operand);
            }
            other => return Err(cli::invalid("cp", other)),
        }
    }
    let Some(dst) = last.filter(|_| count >= 2) else {
        return Err(match first {
            Some(src) => diag::missing_destination_operand("cp", src),
            None => diag::missing_file_operand("cp"),
        });
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            diag::cannot("cp", "stat", dst, e);
            return Ok(ExitCode(1));
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return Err(diag::target_not_directory("cp", dst));
    }

    let mut status = 0;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }

        let joined: String;
        let target: &str = if dst_is_dir {
            joined = join(dst, basename(src));
            &joined
        } else {
            dst
        };

        if src == target {
            let _ = writeln!(Fd(2), "cp: '{src}' and '{target}' are the same file");
            status = 1;
            continue;
        }

        if copy_operand(src, target, &o).is_err() {
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
