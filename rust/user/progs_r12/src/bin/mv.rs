//! `mv SRC... DST` -- see `docs/progs.md`.
//!
//! `hadris-fat`'s `rename` refuses if `DST` already names something, whatever type it is, so the
//! POSIX `mv`(1) behaviors on top of it -- moving into an existing directory, replacing an existing
//! file -- are entirely this program's own layer: probe `DST` with `stat` first, then decide.

#![no_std]
#![no_main]

use abi::errno::{EEXIST, EINVAL, ENAMETOOLONG, ENOENT, ENOTDIR};
use progs::{PathBuf, basename, fail, help, unknown_option, usage};
use userlib::{ATTR_DIRECTORY, ExitCode, rename, stat, unlink};

userlib::entry_with_args!(run);

const USAGE: &str = "mv SRC... DST";
const FLAGS: &[(&str, &str)] = &[];

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. Replaces
/// an existing plain-file target (matching real `mv`(1)/`rename(2)`); refuses (`File exists`)
/// whenever either side of a replacement would be a directory, since a partial replace could
/// orphan a directory's contents.
fn move_one(src: &str, dst: &str, dst_is_dir: bool) -> Result<(), isize> {
    let joined;
    let target: &str = if dst_is_dir {
        joined = PathBuf::join(dst, basename(src)).ok_or(ENAMETOOLONG)?;
        joined.as_str()
    } else {
        dst
    };

    if src == target {
        return Err(EINVAL);
    }

    match stat(target) {
        Err(ENOENT) => {
            let r = rename(src, target);
            if r < 0 { Err(r) } else { Ok(()) }
        }
        Err(e) => Err(e),
        Ok(target_info) => {
            if target_info.attrs & ATTR_DIRECTORY != 0 {
                return Err(EEXIST);
            }
            let src_info = stat(src)?;
            if src_info.attrs & ATTR_DIRECTORY != 0 {
                return Err(EEXIST);
            }
            let u = unlink(target, false);
            if u < 0 {
                return Err(u);
            }
            let r = rename(src, target);
            if r < 0 { Err(r) } else { Ok(()) }
        }
    }
}

fn run(args: userlib::Args) -> ExitCode {
    // First pass: validate operands, count them, and capture the last one as dst.
    let mut count = 0usize;
    let mut dst: Option<&str> = None;
    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("mv", arg);
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
            fail("mv", dst, e);
            return ExitCode(1);
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return usage("mv SRC SRC... DIR  (more than one source requires an existing directory destination)");
    }
    if dst.ends_with('/') && !dst_is_dir {
        fail("mv", dst, ENOTDIR);
        return ExitCode(1);
    }

    let mut status = 0;
    for (i, src) in args.skip(1).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if let Err(e) = move_one(src, dst, dst_is_dir) {
            fail("mv", src, e);
            status = 1;
        }
    }

    ExitCode(status)
}
