//! `cp SRC... DST` -- see `docs/progs.md`. Stage 12's tier replaces the base `cp` (which only ever
//! took exactly two operands, and predates a working directory to resolve a directory destination
//! against) with multi-source and directory-destination support, both of which need the `stat`
//! syscall this stage adds -- r09-r11's older kernels don't have it, so this can't live in the
//! base tier the way `cp`'s core copy loop otherwise would.

#![no_std]
#![no_main]

use core::fmt::Write;

use abi::errno::{EISDIR, ENAMETOOLONG, ENOENT};
use progs::{CHUNK, Fd, PathBuf, basename, diag, help, write_all};
use userlib::{ATTR_DIRECTORY, ExitCode, O_RDONLY, O_WRONLY, close, open, read, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "cp SRC... DST";
const FLAGS: &[(&str, &str)] = &[];

/// Copies `src` to `dst`, reporting in GNU's wording (attributed to whichever side actually failed) and
/// returning `Err(())` if it didn't work. Reads the first chunk *before* creating (and so
/// truncating) `dst`: a source that can't be read at all -- e.g., a directory -- must not destroy
/// an existing destination first.
fn copy_one(src: &str, dst: &str) -> Result<(), ()> {
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

    // Closing is what commits the file's size to disk -- a failure here is a failed copy, and
    // takes priority over an already-recorded read/write error only if there wasn't one.
    let closed = close(output);
    if closed < 0 && status.is_ok() {
        diag::report("cp", "error writing", dst, closed);
        status = Err(());
    }
    status
}

fn run(args: userlib::Args) -> ExitCode {
    let mut count = 0usize;
    let mut first: Option<&str> = None;
    let mut dst: Option<&str> = None;
    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return diag::invalid_option("cp", arg);
        }
        count += 1;
        first.get_or_insert(arg);
        dst = Some(arg);
    }
    let Some(dst) = dst.filter(|_| count >= 2) else {
        return match first {
            Some(src) => diag::missing_destination_operand("cp", src),
            None => diag::missing_file_operand("cp"),
        };
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            diag::cannot("cp", "stat", dst, e);
            return ExitCode(1);
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return diag::target_not_directory("cp", dst);
    }

    let mut status = 0;
    for (i, src) in args.skip(1).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }

        let joined;
        let target: &str = if dst_is_dir {
            match PathBuf::join(dst, basename(src)) {
                Some(p) => {
                    joined = p;
                    joined.as_str()
                }
                None => {
                    diag::cannot("cp", "create regular file", dst, ENAMETOOLONG);
                    status = 1;
                    continue;
                }
            }
        } else {
            dst
        };

        if src == target {
            let _ = writeln!(Fd(2), "cp: '{src}' and '{target}' are the same file");
            status = 1;
            continue;
        }

        if copy_one(src, target).is_err() {
            status = 1;
        }
    }

    ExitCode(status)
}
