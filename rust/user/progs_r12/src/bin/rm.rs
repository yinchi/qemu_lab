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
use progs::{PathBuf, basename, fail, help, unknown_option};
use userlib::{
    ATTR_DIRECTORY, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, close, getdents, open, stat, unlink,
};

userlib::entry_with_args!(run);

const USAGE: &str = "rm [-r] [-f] PATH...";
const FLAGS: &[(&str, &str)] = &[
    ("-r", "remove directories and their contents recursively"),
    ("-f", "ignore nonexistent operands, never prompt"),
];

/// `.`/`..` and the root are refused outright, `-f` included -- not a POSIX-prompt analogue, a
/// hard guard `-f` never bypasses (matches GNU).
fn forbidden(path: &str) -> bool {
    path == "/" || matches!(basename(path), "." | "..")
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
    let (mut recursive, mut force) = (false, false);
    let mut any = false;
    let mut status = 0;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            for flag in arg[1..].chars() {
                match flag {
                    'r' => recursive = true,
                    'f' => force = true,
                    _ => return unknown_option("rm", arg),
                }
            }
            continue;
        }
        any = true;

        if forbidden(arg) {
            fail("rm", arg, abi::errno::EINVAL);
            status = 1;
            continue;
        }

        let info = match stat(arg) {
            Ok(info) => info,
            Err(e @ abi::errno::ENOENT) => {
                if !force {
                    fail("rm", arg, e);
                    status = 1;
                }
                continue;
            }
            Err(e) => {
                fail("rm", arg, e);
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
            fail("rm", arg, e);
            status = 1;
        }
    }

    if !any {
        return progs::usage(USAGE);
    }

    ExitCode(status)
}
