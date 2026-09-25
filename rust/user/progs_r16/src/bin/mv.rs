//! `mv SRC... DST` -- see `docs/progs.md`.
//!
//! `hadris-fat`'s `rename` refuses if `DST` already names something, whatever type it is, so the
//! POSIX `mv`(1) behaviors on top of it -- moving into an existing directory, replacing an existing
//! file -- are entirely this program's own layer: probe `DST` with `stat` first, then decide.

#![no_std]
#![no_main]

use core::fmt::Write;

use abi::errno::{EEXIST, EINVAL, ENOENT, ENOTDIR};
use progs::{Fd, basename, diag, errmsg};
use userlib::{ATTR_DIRECTORY, ExitCode, rename, stat, unlink};
use progs_r12::cli;
use progs_r16::join;

userlib::entry_with_args!(run);

const USAGE: &str = "mv SRC... DST";
const FLAGS: &[(&str, &str)] = &[];

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. Replaces
/// an existing plain-file target (matching real `mv`(1)/`rename(2)`); refuses (`File exists`)
/// whenever either side of a replacement would be a directory, since a partial replace could
/// orphan a directory's contents. Reports its own failures (GNU's wording where it has some) and
/// returns `Err(())` for one.
fn move_one(src: &str, dst: &str, dst_is_dir: bool) -> Result<(), ()> {
    let joined;
    let target: &str = if dst_is_dir {
        joined = join(dst, basename(src));
        &joined
    } else {
        dst
    };

    if src == target {
        let _ = writeln!(Fd(2), "mv: '{src}' and '{target}' are the same file");
        return Err(());
    }

    match try_move(src, target) {
        Ok(()) => Ok(()),
        // The kernel's `rename` refuses moving a directory into itself with EINVAL. That is also its
        // answer to a name FAT cannot hold, so only say "subdirectory" when the paths look like it.
        Err(EINVAL) if target.strip_prefix(src).is_some_and(|rest| rest.starts_with('/')) => {
            let _ = writeln!(Fd(2), "mv: cannot move '{src}' to a subdirectory of itself, '{target}'");
            Err(())
        }
        Err(e) => {
            let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{target}': {}", errmsg(e));
            Err(())
        }
    }
}

/// The move itself, once `target` is settled: probes `target` with `stat`, then renames (replacing
/// a plain-file target first).
fn try_move(src: &str, target: &str) -> Result<(), isize> {
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
    let plain = match cli::plain("mv", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let count = plain.count;
    let first = plain.first;
    let Some(dst) = plain.last.filter(|_| count >= 2) else {
        return match first {
            Some(src) => diag::missing_destination_operand("mv", src),
            None => diag::missing_file_operand("mv"),
        };
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            diag::cannot("mv", "stat", dst, e);
            return ExitCode(1);
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return diag::target_not_directory("mv", dst);
    }
    if dst.ends_with('/') && !dst_is_dir {
        let src = first.unwrap_or(dst);
        let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{dst}': {}", errmsg(ENOTDIR));
        return ExitCode(1);
    }

    let mut status = 0;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if move_one(src, dst, dst_is_dir).is_err() {
            status = 1;
        }
    }

    ExitCode(status)
}
