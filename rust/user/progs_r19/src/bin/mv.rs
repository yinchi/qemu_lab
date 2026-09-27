//! `mv [-f | -i | -n] [-u] [-b] [-v] [-T | -t DIR] SRC... DST` -- see `docs/progs.md`. Stage 19's tier makes `mv` work **between two
//! volumes**, where the kernel's `rename` says `EXDEV`: it then copies and removes; and adds `-i` (ask before replacing), `-u` (only
//! replace an older file), `-b` (keep the replaced file as `NAME~`), `-t DIR` (move into `DIR`) and `-T` (`DST` is never a directory
//! to move into).
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
use alloc::vec::Vec;
use core::fmt::Write;

use abi::errno::{EEXIST, EINVAL, ENOENT, ENOSPC, ENOTDIR, EXDEV};
use getargs::Arg;
use progs::{Fd, basename, diag, errmsg, help};
use progs_r12::cli;
use progs_r18::join;
use progs_r19::copy::{Copier, remove_tree};
use progs_r19::stamp::fat_to_unix;
use userlib::{ATTR_DIRECTORY, ExitCode, read, rename, stat, unlink};

userlib::entry_with_args!(run);

const USAGE: &str = "mv [-f | -i | -n] [-u] [-b] [-v] [-T | -t DIR] SRC... DST";
const FLAGS: &[(&str, &str)] = &[
    ("-f", "replace without asking (the default; the last of -f, -i and -n wins)"),
    ("-i", "ask before replacing an existing target (y to go on)"),
    ("-n", "do not replace an existing target"),
    ("-u", "replace only a target that is older than the source"),
    ("-b", "keep a replaced file as NAME~"),
    ("-t DIR", "move every SRC into the directory DIR"),
    ("-T", "treat DST as a file, never as a directory to move into"),
    ("-v", "print what is being moved"),
];

/// What to do when the target already exists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overwrite {
    Force,
    Interactive,
    NoClobber,
}

struct Options {
    overwrite: Overwrite,
    update: bool,
    backup: bool,
    verbose: bool,
}

/// Asks whether to replace `target`, on standard error, and reads the answer from standard input: yes if it starts with `y`.
fn confirm(target: &str) -> bool {
    let _ = write!(Fd(2), "mv: overwrite '{target}'? ");
    let mut line = [0u8; 64];
    let n = read(0, &mut line);
    n > 0 && line[..n as usize].iter().find(|b| !b.is_ascii_whitespace()).is_some_and(|b| b.eq_ignore_ascii_case(&b'y'))
}

/// Whether `src` was modified after `target` (FAT's 2-second steps: the same stamp is not newer).
fn newer(src: &str, target: &str) -> bool {
    let (Ok(s), Ok(t)) = (stat(src), stat(target)) else { return true };
    match (fat_to_unix(s.modified_date, s.modified_time), fat_to_unix(t.modified_date, t.modified_time)) {
        (Some(s), Some(t)) => s > t,
        _ => true,
    }
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
    // What is there already decides whether, and how, to go on -- for a file; a directory in the way is `try_move`'s to refuse.
    let mut backup: Option<String> = None;
    if let Ok(existing) = stat(target)
        && existing.attrs & ATTR_DIRECTORY == 0
    {
        if o.update && !newer(src, target) {
            return Ok(()); // as GNU's `mv -u`: nothing to do is not a failure, and not worth a word
        }
        match o.overwrite {
            // As GNU's `mv -n` does: say so, and count it as a failure.
            Overwrite::NoClobber => {
                let _ = writeln!(Fd(2), "mv: not replacing '{target}'");
                return Err(());
            }
            Overwrite::Interactive if !confirm(target) => return Err(()),
            _ => {}
        }
        if o.backup {
            let kept = alloc::format!("{target}~");
            let r = rename(target, &kept);
            if r < 0 {
                diag::cannot("mv", "backup", target, r);
                return Err(());
            }
            backup = Some(kept);
        }
    }

    match try_move(src, target) {
        Ok(()) => {
            if o.verbose {
                match backup {
                    Some(kept) => {
                        let _ = writeln!(Fd(1), "renamed '{src}' -> '{target}' (backup: '{kept}')");
                    }
                    None => {
                        let _ = writeln!(Fd(1), "renamed '{src}' -> '{target}'");
                    }
                }
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
    let mut o = Options { overwrite: Overwrite::Force, update: false, backup: false, verbose: false };
    let mut target_dir: Option<&str> = None;
    let mut no_target = false;
    let mut operands: Vec<&str> = Vec::new();
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("mv", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('f') | Arg::Long("force") => o.overwrite = Overwrite::Force,
            Arg::Short('i') | Arg::Long("interactive") => o.overwrite = Overwrite::Interactive,
            Arg::Short('n') | Arg::Long("no-clobber") => o.overwrite = Overwrite::NoClobber,
            Arg::Short('u') | Arg::Long("update") => o.update = true,
            Arg::Short('b') | Arg::Long("backup") => o.backup = true,
            Arg::Short('v') | Arg::Long("verbose") => o.verbose = true,
            Arg::Short('T') | Arg::Long("no-target-directory") => no_target = true,
            Arg::Short('t') | Arg::Long("target-directory") => target_dir = Some(cli::value("mv", &mut opts)?),
            Arg::Positional(operand) => operands.push(operand),
            other => return Err(cli::invalid("mv", other)),
        }
    }
    if target_dir.is_some() && no_target {
        let _ = writeln!(Fd(2), "mv: cannot combine --target-directory (-t) and --no-target-directory (-T)");
        return Err(ExitCode(1));
    }

    // Where they go, and which operands are the things to move.
    let (dst, sources): (&str, &[&str]) = match target_dir {
        Some(dir) => {
            if operands.is_empty() {
                return Err(diag::missing_file_operand("mv"));
            }
            match stat(dir) {
                Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {}
                Ok(_) => {
                    let _ = writeln!(Fd(2), "mv: target directory '{dir}': {}", errmsg(ENOTDIR));
                    return Ok(ExitCode(1));
                }
                Err(e) => {
                    let _ = writeln!(Fd(2), "mv: target directory '{dir}': {}", errmsg(e));
                    return Ok(ExitCode(1));
                }
            }
            (dir, &operands[..])
        }
        None => {
            let Some((&dst, sources)) = operands.split_last().filter(|(_, sources)| !sources.is_empty()) else {
                return Err(match operands.first() {
                    Some(src) => diag::missing_destination_operand("mv", src),
                    None => diag::missing_file_operand("mv"),
                });
            };
            if no_target && sources.len() > 1 {
                return Err(diag::extra_operand("mv", sources[1]));
            }
            (dst, sources)
        }
    };

    let dst_is_dir = target_dir.is_some()
        || (!no_target
            && match stat(dst) {
                Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
                Err(ENOENT) => false,
                Err(e) => {
                    diag::cannot("mv", "stat", dst, e);
                    return Ok(ExitCode(1));
                }
            });

    if sources.len() > 1 && !dst_is_dir {
        return Err(diag::target_not_directory("mv", dst));
    }
    if !no_target && dst.ends_with('/') && !dst_is_dir {
        let _ = writeln!(Fd(2), "mv: cannot move '{}' to '{dst}': {}", sources[0], errmsg(ENOTDIR));
        return Ok(ExitCode(1));
    }

    let mut status = 0;
    for src in sources {
        if move_one(src, dst, dst_is_dir, &o).is_err() {
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
