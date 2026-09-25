//! `ls [-F] [-l] [dir...]` -- see `docs/progs.md`. Stage 12's tier listed the working directory (not the root) when
//! given no operand; from Stage 16 the entries of each directory are also **sorted by name** (bytewise, the C locale's
//! order, as POSIX `ls` sorts), which needs the whole listing in memory -- the user heap makes that possible. Before,
//! entries came out in on-disk order.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, diag, help};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, ExitCode};
use getargs::Arg;
use progs_r12::cli;
use progs_r16::{ReadDirError, read_dir};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [-F] [-l] [dir...]";
const FLAGS: &[(&str, &str)] = &[
    ("-F", "append / to directories and * to executable files"),
    ("-l", "long format: d/w/x flags, size, name"),
];

/// Lists one directory, sorted by name (bytewise, the C locale's order). Returns `Err(())` (already reported)
/// on failure.
fn list_one(dir: &str, classify: bool, long: bool) -> Result<(), ()> {
    let mut entries = match read_dir(dir) {
        Ok(entries) => entries,
        Err(ReadDirError::Open(e)) => {
            // GNU: a name that does not exist is "cannot access"; a directory that will not open, "cannot open directory".
            if e == abi::errno::ENOENT {
                diag::cannot("ls", "access", dir, e);
            } else {
                diag::cannot("ls", "open directory", dir, e);
            }
            return Err(());
        }
        Err(ReadDirError::Read(e)) => {
            // Opened, but not a directory (this `ls` cannot list a plain file): GNU calls that "cannot open directory".
            if e == abi::errno::ENOTDIR {
                diag::cannot("ls", "open directory", dir, e);
            } else {
                diag::report("ls", "reading directory", dir, e);
            }
            return Err(());
        }
    };
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

    for ent in &entries {
        if long {
            let type_c = if ent.attrs & ATTR_DIRECTORY != 0 { 'd' } else { '-' };
            let w_c = if ent.attrs & ATTR_READ_ONLY != 0 { '-' } else { 'w' };
            let x_c = if ent.attrs & ATTR_EXEC != 0 { 'x' } else { '-' };
            let _ = writeln!(Fd(1), "{type_c}{w_c}{x_c} {:>10} {}", ent.size, ent.name);
        } else {
            let suffix = match (classify, ent.attrs) {
                (true, a) if a & ATTR_DIRECTORY != 0 => "/",
                (true, a) if a & ATTR_EXEC != 0 => "*",
                _ => "",
            };
            let _ = writeln!(Fd(1), "{}{suffix}", ent.name);
        }
    }
    Ok(())
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(list(args))
}

fn list(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut classify = false;
    let mut long = false;
    let mut n_dirs = 0usize;

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("ls", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('F') => classify = true,
            Arg::Short('l') => long = true,
            // One entry per line is all this `ls` ever does, so `-1` is accepted and changes nothing.
            Arg::Short('1') => {}
            Arg::Positional(_) => n_dirs += 1,
            other => return Err(cli::invalid("ls", other)),
        }
    }

    let mut status = 0;

    if n_dirs == 0 {
        if list_one(".", classify, long).is_err() {
            status = 1;
        }
        return Ok(ExitCode(status));
    }

    let mut first = true;
    for arg in cli::operands(args) {
        if n_dirs > 1 {
            if !first {
                let _ = writeln!(Fd(1));
            }
            let _ = writeln!(Fd(1), "{arg}:");
        }
        first = false;
        if list_one(arg, classify, long).is_err() {
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
