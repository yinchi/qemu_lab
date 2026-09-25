//! `rm [-r] [-f] PATH...` -- see `docs/progs.md`.
//!
//! `-r` drills into a directory before removing it: since `hadris-fat`'s `delete` only removes a
//! file or an *empty* directory, recursion is ours. Each directory is listed whole (`progs_r16::read_dir`, which
//! closes its descriptor before returning), then its entries are removed -- subdirectories first, recursively -- and
//! finally the directory itself. Stages 12-15, with no heap to hold a listing, instead opened the directory, read one
//! record, closed it, removed that entry and reopened, until it came up empty.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, basename, diag, help};
use userlib::{ATTR_DIRECTORY, ExitCode, stat, unlink};
use getargs::Arg;
use progs_r12::cli;
use progs_r16::{join, read_dir};

userlib::entry_with_args!(run);

const USAGE: &str = "rm [-r] [-f] PATH...";
const FLAGS: &[(&str, &str)] = &[
    ("-r", "remove directories and their contents recursively"),
    ("-f", "ignore nonexistent operands, never prompt"),
];

/// `.`/`..` and the root are refused outright, `-f` included -- not a POSIX-prompt analogue, a
/// hard guard `-f` never bypasses (matches GNU). Returns the message to print (GNU's wording), if
/// `path` is one of them.
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

/// Removes the directory at `path` and everything under it: lists it (`read_dir`, which holds no descriptor
/// afterwards), removes each entry -- recursing into subdirectories first -- and then `path` itself.
fn remove_recursive(path: &str) -> Result<(), isize> {
    for ent in read_dir(path).map_err(|e| e.errno())? {
        let child = join(path, &ent.name);
        if ent.attrs & ATTR_DIRECTORY != 0 {
            remove_recursive(&child)?;
        } else {
            let r = unlink(&child, false);
            if r < 0 {
                return Err(r);
            }
        }
    }
    let r = unlink(path, true);
    if r < 0 { Err(r) } else { Ok(()) }
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(remove(args))
}

fn remove(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let (mut recursive, mut force) = (false, false);
    let mut any = false;
    let mut status = 0;

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("rm", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('r') | Arg::Short('R') => recursive = true,
            Arg::Short('f') => force = true,
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

        let result = if info.attrs & ATTR_DIRECTORY != 0 {
            if !recursive {
                Err(abi::errno::EISDIR)
            } else {
                remove_recursive(arg)
            }
        } else {
            let r = unlink(arg, false);
            if r < 0 { Err(r) } else { Ok(()) }
        };

        if let Err(e) = result {
            diag::cannot("rm", "remove", arg, e);
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
