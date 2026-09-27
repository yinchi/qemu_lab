//! `mv [-f] [-n] [-v] SRC... DST` -- see `docs/progs.md`. Stage 19's tier makes `mv` work **between two volumes**, where
//! the kernel's `rename` says `EXDEV`: it then copies and removes.
//!
//! On one volume nothing changed: `DST` is probed with `stat`, `rename` does the move, and it replaces an existing file
//! itself (Stage 18 removed the target first, which across volumes lost it when the rename then failed). When `rename`
//! is `EXDEV`, the source -- a file or a whole directory tree -- is copied and only then removed, in POSIX's order and not
//! GNU's remove-as-you-go, so nothing of the source is touched until all of it has been copied and every file's size
//! checked:
//!
//! 1. the copy is made under a hidden name in the destination's own directory, `.mv-partial` (or `.mv-partial.1`, `.2`, ...
//!    for the first that is free), keeping modify times and the executable and read-only bits (`progs_r19::copy`);
//! 2. it is renamed into place -- the kernel's `rename` replacing an existing file, so the old one is never truncated first;
//! 3. only then is the source removed.
//!
//! A failure in step 1 removes the hidden copy and leaves the source whole; a failure in step 3 leaves the destination
//! complete and says which entries of the source it could not remove. A crash leaves a dot-file to `rm -r`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write;

use abi::errno::{EEXIST, EINVAL, ENOENT, ENOSPC, ENOTDIR, EXDEV};
use getargs::Arg;
use progs::{Fd, basename, diag, errmsg, help};
use progs_r12::cli;
use progs_r18::join;
use progs_r19::copy::{Copier, remove_tree};
use userlib::{ATTR_DIRECTORY, ExitCode, rename, stat, unlink};

userlib::entry_with_args!(run);

const USAGE: &str = "mv [-f] [-n] [-v] SRC... DST";
const FLAGS: &[(&str, &str)] = &[
    ("-f", "accepted and ignored: nothing here prompts"),
    ("-n", "do not replace an existing target"),
    ("-v", "print what is being moved"),
];

struct Options {
    no_clobber: bool,
    verbose: bool,
}

/// Why a move failed: the kernel's error, for `move_one` to word, or something already reported.
enum Fail {
    Errno(isize),
    Reported,
}

/// The name of the hidden copy: `.mv-partial` in `target`'s directory, or the first of `.mv-partial.1`, `.2`, ... that is
/// not taken. `None` if a thousand are (a directory full of leftovers).
fn partial_path(target: &str) -> Option<String> {
    let dir = match target.rfind('/') {
        Some(0) => "/",
        Some(at) => &target[..at],
        None => "",
    };
    let named = |name: &str| if dir.is_empty() { String::from(name) } else { join(dir, name) };
    let first = named(".mv-partial");
    if stat(&first).is_err() {
        return Some(first);
    }
    (1..1000).map(|n| named(&alloc::format!(".mv-partial.{n}"))).find(|path| stat(path).is_err())
}

/// The move between two volumes, described above. Reports its own failures (as `mv`, in GNU's wording).
fn move_across(src: &str, target: &str) -> Result<(), Fail> {
    let info = stat(src).map_err(Fail::Errno)?;
    let is_dir = info.attrs & ATTR_DIRECTORY != 0;
    let Some(partial) = partial_path(target) else {
        let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{target}': {}", errmsg(ENOSPC));
        return Err(Fail::Reported);
    };
    let copier = Copier { prog: "mv", recursive: true, no_clobber: false, verbose: false, preserve: true };

    let copied = if is_dir { copier.tree(src, &partial) } else { copier.file(src, &partial) };
    if copied.is_err() {
        let _ = remove_tree("mv", &partial, false);
        return Err(Fail::Reported);
    }
    let renamed = rename(&partial, target);
    if renamed < 0 {
        let _ = remove_tree("mv", &partial, false);
        let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{target}': {}", errmsg(renamed));
        return Err(Fail::Reported);
    }
    // The destination is complete: what is left is taking the source away.
    let removed = if is_dir {
        remove_tree("mv", src, true)
    } else {
        let r = unlink(src, false);
        if r < 0 {
            diag::cannot("mv", "remove", src, r);
            Err(progs_r19::copy::Reported)
        } else {
            Ok(())
        }
    };
    if removed.is_err() {
        let _ = writeln!(Fd(2), "mv: '{target}' is complete, but '{src}' could not be removed entirely");
        return Err(Fail::Reported);
    }
    Ok(())
}

/// `rename`, and -- if the two are on different volumes -- the copy and removal.
fn rename_or_move_across(src: &str, target: &str) -> Result<(), Fail> {
    let r = rename(src, target);
    if r == EXDEV {
        return move_across(src, target);
    }
    if r < 0 { Err(Fail::Errno(r)) } else { Ok(()) }
}

/// Moves `src` to `dst`, or -- if `dst_is_dir` -- into `dst` under `src`'s own basename. Replaces an existing plain-file
/// target (matching real `mv`(1)/`rename(2)`) unless `-n`; refuses (`File exists`) whenever either side of a replacement
/// would be a directory, since a partial replace could orphan a directory's contents. Reports its own failures (GNU's
/// wording where it has some) and returns `Err(())` for one.
fn move_one(src: &str, dst: &str, dst_is_dir: bool, o: &Options) -> Result<(), ()> {
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
    if o.no_clobber && stat(target).is_ok() {
        // As GNU's `mv -n` does: say so, and count it as a failure.
        let _ = writeln!(Fd(2), "mv: not replacing '{target}'");
        return Err(());
    }

    match try_move(src, target) {
        Ok(()) => {
            if o.verbose {
                let _ = writeln!(Fd(1), "renamed '{src}' -> '{target}'");
            }
            Ok(())
        }
        Err(Fail::Reported) => Err(()),
        // The kernel's `rename` refuses moving a directory into itself with EINVAL. That is also its answer to a name
        // FAT cannot hold, so only say "subdirectory" when the paths look like it.
        Err(Fail::Errno(EINVAL)) if target.strip_prefix(src).is_some_and(|rest| rest.starts_with('/')) => {
            let _ = writeln!(Fd(2), "mv: cannot move '{src}' to a subdirectory of itself, '{target}'");
            Err(())
        }
        Err(Fail::Errno(e)) => {
            let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{target}': {}", errmsg(e));
            Err(())
        }
    }
}

/// The move itself, once `target` is settled: probes `target` with `stat`, refuses what would replace or be replaced by a
/// directory, and otherwise renames (the kernel replaces a plain-file target), falling back to a copy across volumes.
fn try_move(src: &str, target: &str) -> Result<(), Fail> {
    match stat(target) {
        Err(ENOENT) => rename_or_move_across(src, target),
        Err(e) => Err(Fail::Errno(e)),
        Ok(target_info) => {
            if target_info.attrs & ATTR_DIRECTORY != 0 {
                return Err(Fail::Errno(EEXIST));
            }
            let src_info = stat(src).map_err(Fail::Errno)?;
            if src_info.attrs & ATTR_DIRECTORY != 0 {
                return Err(Fail::Errno(EEXIST));
            }
            rename_or_move_across(src, target)
        }
    }
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(mv(args))
}

fn mv(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut o = Options { no_clobber: false, verbose: false };
    let (mut count, mut first, mut last) = (0usize, None, None);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("mv", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('f') | Arg::Long("force") => {}
            Arg::Short('n') | Arg::Long("no-clobber") => o.no_clobber = true,
            Arg::Short('v') | Arg::Long("verbose") => o.verbose = true,
            Arg::Positional(operand) => {
                count += 1;
                first.get_or_insert(operand);
                last = Some(operand);
            }
            other => return Err(cli::invalid("mv", other)),
        }
    }
    let Some(dst) = last.filter(|_| count >= 2) else {
        return Err(match first {
            Some(src) => diag::missing_destination_operand("mv", src),
            None => diag::missing_file_operand("mv"),
        });
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            diag::cannot("mv", "stat", dst, e);
            return Ok(ExitCode(1));
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return Err(diag::target_not_directory("mv", dst));
    }
    if dst.ends_with('/') && !dst_is_dir {
        let src = first.unwrap_or(dst);
        let _ = writeln!(Fd(2), "mv: cannot move '{src}' to '{dst}': {}", errmsg(ENOTDIR));
        return Ok(ExitCode(1));
    }

    let mut status = 0;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }
        if move_one(src, dst, dst_is_dir, &o).is_err() {
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
