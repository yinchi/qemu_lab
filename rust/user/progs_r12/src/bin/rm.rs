//! `rm [-r] [-f] PATH...` -- see `docs/progs.md`.
//!
//! `-r` drills into a directory before removing it: since `hadris-fat`'s `delete` only removes a
//! file or an *empty* directory, recursion is ours. Implemented with no fd held across recursion
//! (open the directory, read one record, close, recurse into it first if it's a directory, delete
//! it, repeat) until the directory reports empty, then remove it -- reopening always sees a
//! different (or no) first record next time, since each iteration's delete shrinks the listing.

#![no_std]
#![no_main]

use abi::errno::{EIO, ENAMETOOLONG};
use core::fmt::Write;

use progs::{Fd, PathBuf, basename, diag, help};
use userlib::{
    ATTR_DIRECTORY, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, close, getdents, open, stat, unlink,
};
use getargs::Arg;
use progs_r12::cli;

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

/// Empties the directory at `path` by repeatedly reading and deleting its first entry (recursing
/// first if that entry is itself a directory), then removing `path` itself once it's empty.
fn remove_recursive(path: &str) -> Result<(), isize> {
    loop {
        let fd = open(path, O_RDONLY);
        if fd < 0 {
            return Err(fd);
        }
        let fd = fd as usize;
        let mut buf = [0u8; DIRENT_SIZE];
        let n = getdents(fd, &mut buf);
        close(fd);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            break; // empty
        }
        let Some(ent) = DirEnt::parse(&buf[..n as usize]) else {
            return Err(EIO);
        };
        let Some(child) = PathBuf::join(path, ent.name) else {
            return Err(ENAMETOOLONG);
        };
        if ent.attrs & ATTR_DIRECTORY != 0 {
            // `remove_recursive` already removes `child` itself once it's empty (see its own
            // trailing `unlink` below) -- unlinking it again here would double-delete.
            remove_recursive(child.as_str())?;
        } else {
            let r = unlink(child.as_str(), false);
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
