//! `find [PATH...] [-name GLOB] [-type f|d] [-mindepth N] [-maxdepth N]` -- see `docs/progs.md`. Prints each path at or under
//! each PATH (default `.`) that passes every test given, one per line. Paths are printed as GNU prints them: the start
//! path as written, then `dir/name` for what is under it. Unlike GNU's, the entries of a directory are visited in name order.
//!
//! The tests: `-name GLOB` (the last component of the path matches the pattern: `*`, `?`, `[a-z]`), `-type f` or `-type d`
//! (a file or a directory), `-mindepth N` and `-maxdepth N` (how far below the start path: 0 is the start path itself).
//! There is no `-exec`: a program cannot start another.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::string::String;
use alloc::vec::Vec;
use progs::{Fd, basename, diag, help};
use progs_r18::glob::matches;
use progs_r18::{join, read_dir};
use userlib::{ATTR_DIRECTORY, ExitCode, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "find [PATH...] [-name GLOB] [-type f|d] [-mindepth N] [-maxdepth N]";
const FLAGS: &[(&str, &str)] = &[
    ("-name GLOB", "the last component matches GLOB (* ? [a-z])"),
    ("-type f|d", "a file, or a directory"),
    ("-mindepth N", "at least N levels below the start path"),
    ("-maxdepth N", "at most N levels below the start path"),
];

struct Tests {
    name: Option<&'static str>,
    dirs: Option<bool>,
    min_depth: usize,
    max_depth: usize,
}

fn show(path: &str, is_dir: bool, depth: usize, t: &Tests) {
    if depth < t.min_depth {
        return;
    }
    if t.dirs.is_some_and(|want_dir| want_dir != is_dir) {
        return;
    }
    if t.name.is_some_and(|pattern| !matches(pattern, basename(path))) {
        return;
    }
    let _ = writeln!(Fd(1), "{path}");
}

/// Visits `path` (already known to be a directory or not) and, if it is one and there is depth left, everything under it.
/// `Err(())` if anything under it could not be read (reported).
fn walk(path: &str, is_dir: bool, depth: usize, t: &Tests) -> Result<(), ()> {
    show(path, is_dir, depth, t);
    if !is_dir || depth >= t.max_depth {
        return Ok(());
    }
    let mut entries = match read_dir(path) {
        Ok(entries) => entries,
        Err(e) => {
            diag::report("find", "cannot read directory", path, e.errno());
            return Err(());
        }
    };
    entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let mut result = Ok(());
    for ent in entries {
        let child = join(path, &ent.name);
        if walk(&child, ent.attrs & ATTR_DIRECTORY != 0, depth + 1, t).is_err() {
            result = Err(());
        }
    }
    result
}

fn number(what: &str, text: &str) -> Result<usize, ExitCode> {
    text.parse().map_err(|_| {
        let _ = writeln!(Fd(2), "find: invalid argument '{text}' to '{what}'");
        ExitCode(1)
    })
}

fn run(args: userlib::Args) -> ExitCode {
    let mut args = args.skip(1).peekable();
    if args.peek() == Some(&"--help") {
        return help(USAGE, FLAGS);
    }
    // Start paths first, then the tests: the first argument that begins with a dash ends the paths.
    let mut paths: Vec<&'static str> = Vec::new();
    while let Some(&arg) = args.peek() {
        if arg.starts_with('-') && arg.len() > 1 {
            break;
        }
        paths.push(arg);
        args.next();
    }
    if paths.is_empty() {
        paths.push(".");
    }

    let mut t = Tests { name: None, dirs: None, min_depth: 0, max_depth: usize::MAX };
    while let Some(predicate) = args.next() {
        let mut value = |what: &str| -> Result<&'static str, ExitCode> {
            args.next().ok_or_else(|| {
                let _ = writeln!(Fd(2), "find: missing argument to '{what}'");
                ExitCode(1)
            })
        };
        let result = match predicate {
            "-name" => value("-name").map(|p| t.name = Some(p)),
            "-type" => value("-type").and_then(|kind| match kind {
                "f" | "d" => {
                    t.dirs = Some(kind == "d");
                    Ok(())
                }
                other => {
                    let _ = writeln!(Fd(2), "find: Unknown argument to -type: {other}");
                    Err(ExitCode(1))
                }
            }),
            "-mindepth" => value("-mindepth").and_then(|n| number("-mindepth", n)).map(|n| t.min_depth = n),
            "-maxdepth" => value("-maxdepth").and_then(|n| number("-maxdepth", n)).map(|n| t.max_depth = n),
            "-print" => Ok(()),
            other => {
                let _ = writeln!(Fd(2), "find: unknown predicate '{other}'");
                Err(ExitCode(1))
            }
        };
        if let Err(status) = result {
            return status;
        }
    }

    let mut status = 0;
    for path in paths {
        let start = String::from(path);
        match stat(&start) {
            Ok(info) => {
                if walk(&start, info.attrs & ATTR_DIRECTORY != 0, 0, &t).is_err() {
                    status = 1;
                }
            }
            Err(e) => {
                let _ = writeln!(Fd(2), "find: '{path}': {}", progs::errmsg(e));
                status = 1;
            }
        }
    }
    ExitCode(status)
}
