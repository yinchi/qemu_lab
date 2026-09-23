//! `cp SRC... DST` -- see `docs/progs.md`. Stage 12's tier replaces the base `cp` (which only ever
//! took exactly two operands, and predates a working directory to resolve a directory destination
//! against) with multi-source and directory-destination support, both of which need the `stat`
//! syscall this stage adds -- r09-r11's older kernels don't have it, so this can't live in the
//! base tier the way `cp`'s core copy loop otherwise would.

#![no_std]
#![no_main]

use abi::errno::{ENAMETOOLONG, ENOENT};
use progs::{CHUNK, PathBuf, basename, fail, help, unknown_option, usage, write_all};
use userlib::{ATTR_DIRECTORY, ExitCode, O_RDONLY, O_WRONLY, close, open, read, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "cp SRC... DST";
const FLAGS: &[(&str, &str)] = &[];

/// Copies `src` to `dst`, reporting via `fail` (attributed to whichever side actually failed) and
/// returning `Err(())` if it didn't work. Reads the first chunk *before* creating (and so
/// truncating) `dst`: a source that can't be read at all -- e.g., a directory -- must not destroy
/// an existing destination first.
fn copy_one(src: &str, dst: &str) -> Result<(), ()> {
    let input = open(src, O_RDONLY);
    if input < 0 {
        fail("cp", src, input);
        return Err(());
    }
    let input = input as usize;

    let mut buf = [0u8; CHUNK];
    let first = read(input, &mut buf);
    if first < 0 {
        fail("cp", src, first);
        close(input);
        return Err(());
    }

    let output = open(dst, O_WRONLY);
    if output < 0 {
        fail("cp", dst, output);
        close(input);
        return Err(());
    }
    let output = output as usize;

    let mut status = Ok(());
    let mut n = first;
    while n > 0 {
        if let Err(e) = write_all(output, &buf[..n as usize]) {
            fail("cp", dst, e);
            status = Err(());
            break;
        }
        n = read(input, &mut buf);
        if n < 0 {
            fail("cp", src, n);
            status = Err(());
            break;
        }
    }
    close(input);

    // Closing is what commits the file's size to disk -- a failure here is a failed copy, and
    // takes priority over an already-recorded read/write error only if there wasn't one.
    let closed = close(output);
    if closed < 0 && status.is_ok() {
        fail("cp", dst, closed);
        status = Err(());
    }
    status
}

fn run(args: userlib::Args) -> ExitCode {
    let mut count = 0usize;
    let mut dst: Option<&str> = None;
    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("cp", arg);
        }
        count += 1;
        dst = Some(arg);
    }
    let Some(dst) = dst.filter(|_| count >= 2) else {
        return usage(USAGE);
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            fail("cp", dst, e);
            return ExitCode(1);
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return usage("cp SRC SRC... DIR  (more than one source requires an existing directory destination)");
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
                    fail("cp", src, ENAMETOOLONG);
                    status = 1;
                    continue;
                }
            }
        } else {
            dst
        };

        if src == target {
            fail("cp", src, abi::errno::EINVAL);
            status = 1;
            continue;
        }

        if copy_one(src, target).is_err() {
            status = 1;
        }
    }

    ExitCode(status)
}
