//! `mkdir [-p] [-v] DIR...` -- see `docs/progs.md`. Stage 18's tier adds `-p` (make the missing parents, and no error for a
//! directory that is already there) and `-v` (say what was created) to the Stage 12 `mkdir`.
//!
//! The kernel's `mkdirat` needs the parent to exist, so `-p` walks the path one prefix at a time, `stat`ing each: a
//! directory is left alone, a missing one is created, a file is an error (`Not a directory` in the middle of the path,
//! `File exists` for the last component).

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use abi::errno::{EEXIST, ENOENT, ENOTDIR};
use alloc::vec::Vec;
use getargs::Arg;
use progs::{Fd, diag, help};
use progs_r12::cli;
use userlib::{ATTR_DIRECTORY, ExitCode, mkdir, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir [-p] [-v] DIR...";
const FLAGS: &[(&str, &str)] = &[
    ("-p", "make missing parent directories, and do not fail if DIR exists as a directory"),
    ("-v", "print a message for each directory created"),
];

fn created(path: &str) {
    let _ = writeln!(Fd(1), "mkdir: created directory '{path}'");
}

/// Creates `path` and every missing parent. `Err` carries the path to report and the error.
fn make_path(path: &str, verbose: bool) -> Result<(), (&str, isize)> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(()); // the root, or a run of slashes: it exists
    }
    let mut ends: Vec<usize> = trimmed.match_indices('/').map(|(i, _)| i).filter(|&i| i > 0).collect();
    ends.push(trimmed.len());
    let last = ends.len() - 1;
    for (k, &end) in ends.iter().enumerate() {
        let prefix = &trimmed[..end];
        if prefix.ends_with('/') {
            continue; // `a//b`
        }
        match stat(prefix) {
            Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {}
            Ok(_) => return Err((prefix, if k == last { EEXIST } else { ENOTDIR })),
            Err(ENOENT) => {
                let r = mkdir(prefix);
                if r < 0 {
                    return Err((prefix, r));
                }
                if verbose {
                    created(prefix);
                }
            }
            Err(e) => return Err((prefix, e)),
        }
    }
    Ok(())
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(make(args))
}

fn make(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let (mut parents, mut verbose) = (false, false);
    let mut any = false;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("mkdir", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('p') | Arg::Long("parents") => parents = true,
            Arg::Short('v') | Arg::Long("verbose") => verbose = true,
            Arg::Positional(_) => any = true,
            other => return Err(cli::invalid("mkdir", other)),
        }
    }
    if !any {
        return Err(diag::missing_operand("mkdir"));
    }

    let mut status = 0;
    for dir in cli::operands(args) {
        let result = if parents {
            make_path(dir, verbose)
        } else {
            let r = mkdir(dir);
            if r < 0 {
                Err((dir, r))
            } else {
                if verbose {
                    created(dir);
                }
                Ok(())
            }
        };
        if let Err((path, e)) = result {
            diag::cannot("mkdir", "create directory", path, e);
            status = 1;
        }
    }
    Ok(ExitCode(status))
}
