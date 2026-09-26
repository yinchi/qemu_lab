//! `rm [-r] [-f] [-v] [-d] PATH...` -- see `docs/progs.md`. Stage 18's tier adds `-v` (say what was removed) and `-d`
//! (remove an empty directory) to the Stage 16 `rm`.
//!
//! `-r` drills into a directory before removing it: since `hadris-fat`'s `delete` only removes a file or an *empty*
//! directory, recursion is ours. Each directory is listed whole (`read_dir`, which closes its descriptor before
//! returning), its entries are removed in name order -- subdirectories first, recursively -- and finally the directory.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use getargs::Arg;
use progs::{Fd, basename, diag, help};
use progs_r12::cli;
use progs_r18::{join, read_dir};
use userlib::{ATTR_DIRECTORY, ExitCode, stat, unlink};

userlib::entry_with_args!(run);

const USAGE: &str = "rm [-r] [-f] [-v] [-d] PATH...";
const FLAGS: &[(&str, &str)] = &[
    ("-r", "remove directories and their contents recursively"),
    ("-f", "ignore nonexistent operands, never prompt"),
    ("-v", "print a message for each file removed"),
    ("-d", "remove empty directories"),
];

/// `.`/`..` and the root are refused outright, `-f` included -- not a POSIX-prompt analogue, a hard guard `-f` never
/// bypasses (matches GNU). Returns why `path` is refused, if it is.
fn forbidden(path: &str, recursive: bool) -> Option<Message> {
    if matches!(basename(path), "." | "..") {
        Some(Message::DotDirectory)
    } else if path == "/" {
        Some(if recursive { Message::DangerousRoot } else { Message::RootIsDirectory })
    } else {
        None
    }
}

enum Message {
    DotDirectory,
    DangerousRoot,
    RootIsDirectory,
}

/// Prints why `path` was refused, as GNU `rm` words it.
fn refuse(path: &str, why: Message) {
    let _ = match why {
        Message::DotDirectory => {
            writeln!(Fd(2), "rm: refusing to remove '.' or '..' directory: skipping '{path}'")
        }
        Message::DangerousRoot => writeln!(Fd(2), "rm: it is dangerous to operate recursively on '/'"),
        Message::RootIsDirectory => writeln!(Fd(2), "rm: cannot remove '/': Is a directory"),
    };
}

fn removed(path: &str, is_dir: bool, verbose: bool) {
    if verbose {
        let _ = if is_dir {
            writeln!(Fd(1), "removed directory '{path}'")
        } else {
            writeln!(Fd(1), "removed '{path}'")
        };
    }
}

/// Removes the directory at `path` and everything under it: lists it, removes each entry (in name order) -- recursing into
/// subdirectories first -- and then `path` itself. `Err` carries the path that failed and why.
fn remove_recursive(path: &str, verbose: bool) -> Result<(), (alloc::string::String, isize)> {
    let mut entries = read_dir(path).map_err(|e| (alloc::string::String::from(path), e.errno()))?;
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    for ent in entries {
        let child = join(path, &ent.name);
        if ent.attrs & ATTR_DIRECTORY != 0 {
            remove_recursive(&child, verbose)?;
        } else {
            let r = unlink(&child, false);
            if r < 0 {
                return Err((child, r));
            }
            removed(&child, false, verbose);
        }
    }
    let r = unlink(path, true);
    if r < 0 {
        return Err((alloc::string::String::from(path), r));
    }
    removed(path, true, verbose);
    Ok(())
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(remove(args))
}

fn remove(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let (mut recursive, mut force, mut verbose, mut empty_dirs) = (false, false, false, false);
    let mut any = false;
    let mut status = 0;

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("rm", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => recursive = true,
            Arg::Short('f') | Arg::Long("force") => force = true,
            Arg::Short('v') | Arg::Long("verbose") => verbose = true,
            Arg::Short('d') | Arg::Long("dir") => empty_dirs = true,
            Arg::Positional(_) => any = true,
            other => return Err(cli::invalid("rm", other)),
        }
    }
    if !any {
        return Err(diag::missing_operand("rm"));
    }

    for arg in cli::operands(args) {
        if let Some(why) = forbidden(arg, recursive) {
            refuse(arg, why);
            status = 1;
            continue;
        }

        let info = match stat(arg) {
            Ok(info) => info,
            Err(e @ abi::errno::ENOENT) => {
                if !force {
                    diag::cannot("rm", "remove", arg, e);
                    status = 1;
                }
                continue;
            }
            Err(e) => {
                diag::cannot("rm", "remove", arg, e);
                status = 1;
                continue;
            }
        };

        let is_dir = info.attrs & ATTR_DIRECTORY != 0;
        let result = if is_dir && recursive {
            remove_recursive(arg, verbose)
        } else if is_dir && !empty_dirs {
            Err((alloc::string::String::from(arg), abi::errno::EISDIR))
        } else {
            let r = unlink(arg, is_dir);
            if r < 0 {
                Err((alloc::string::String::from(arg), r))
            } else {
                removed(arg, is_dir, verbose);
                Ok(())
            }
        };

        if let Err((path, e)) = result {
            diag::cannot("rm", "remove", &path, e);
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
