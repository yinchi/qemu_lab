//! `ls [-1] [-a] [-d] [-F] [-h] [-l] [-R] [-r] [-S] [-t] [FILE...]` -- see `docs/progs.md`. Stage 18's tier extends the
//! Stage 16 `ls` (whose entries are sorted by name, bytewise) with:
//! - **names starting with `.` are hidden** unless `-a` (as GNU does); the kernel never returns `.` and `..`, so `-a` adds only
//!   dot-named files;
//! - `-d` (list a directory operand itself, not its contents) and, always, **a file operand is listed as itself**;
//! - `-R` (recurse, `dir:` headers), `-r` (reverse the order), `-t` (newest modify time first), `-S` (largest first),
//!   `-h` (sizes as `1.5K`, with `-l`).
//!
//! Only the FAT fields there are: `-l` shows the `d`/`w`/`x` flags and the size, no owner and no time.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use alloc::string::String;
use alloc::vec::Vec;
use getargs::Arg;
use progs::{Fd, diag, help};
use progs_r12::cli;
use progs_r18::human::human_size;
use progs_r18::{ReadDirError, join, read_dir};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, ExitCode, Stat, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [-1] [-a] [-d] [-F] [-h] [-l] [-R] [-r] [-S] [-t] [FILE...]";
const FLAGS: &[(&str, &str)] = &[
    ("-a", "show names starting with '.'"),
    ("-d", "list directories themselves, not their contents"),
    ("-F", "append / to directories and * to executable files"),
    ("-h", "with -l, sizes like 1.5K"),
    ("-l", "long format: d/w/x flags, size, name"),
    ("-R", "list subdirectories recursively"),
    ("-r", "reverse the order"),
    ("-S", "sort by size, largest first"),
    ("-t", "sort by modify time, newest first"),
];

#[derive(Default)]
struct Options {
    all: bool,
    dir_only: bool,
    classify: bool,
    human: bool,
    long: bool,
    recursive: bool,
    reverse: bool,
    by_size: bool,
    by_time: bool,
}

/// One line of a listing.
struct Item {
    name: String,
    size: u32,
    attrs: u8,
    /// The packed FAT modify date and time, which sort in time order; only filled in for `-t`.
    time: u32,
}

impl Item {
    fn is_dir(&self) -> bool {
        self.attrs & ATTR_DIRECTORY != 0
    }
}

fn time_key(info: &Stat) -> u32 {
    (u32::from(info.modified_date) << 16) | u32::from(info.modified_time)
}

/// Sorts `items` as the options say: by name (bytewise, the C locale's order), or by size or time, largest or newest first
/// with the name breaking ties; `-r` reverses whichever it is.
fn sort(items: &mut [Item], o: &Options) {
    items.sort_by(|a, b| {
        let by_name = a.name.as_bytes().cmp(b.name.as_bytes());
        if o.by_size {
            b.size.cmp(&a.size).then(by_name)
        } else if o.by_time {
            b.time.cmp(&a.time).then(by_name)
        } else {
            by_name
        }
    });
    if o.reverse {
        items.reverse();
    }
}

fn print(item: &Item, o: &Options) {
    let mut out = Fd(1);
    if o.long {
        let type_c = if item.is_dir() { 'd' } else { '-' };
        let w_c = if item.attrs & ATTR_READ_ONLY != 0 { '-' } else { 'w' };
        let x_c = if item.attrs & ATTR_EXEC != 0 { 'x' } else { '-' };
        if o.human {
            let _ = write!(out, "{type_c}{w_c}{x_c} {:>6} ", human_size(u64::from(item.size)));
        } else {
            let _ = write!(out, "{type_c}{w_c}{x_c} {:>10} ", item.size);
        }
    }
    let suffix = match (o.classify, item.attrs) {
        (true, a) if a & ATTR_DIRECTORY != 0 => "/",
        (true, a) if a & ATTR_EXEC != 0 => "*",
        _ => "",
    };
    let _ = writeln!(out, "{}{suffix}", item.name);
}

/// Lists the contents of the directory `path`, and with `-R` each subdirectory after it. `Err(())` (already reported)
/// if anything could not be listed; what could be listed still is.
fn list_dir(path: &str, o: &Options) -> Result<(), ()> {
    let entries = match read_dir(path) {
        Ok(entries) => entries,
        Err(ReadDirError::Open(e)) => {
            // GNU: a name that does not exist is "cannot access"; a directory that will not open, "cannot open directory".
            if e == abi::errno::ENOENT {
                diag::cannot("ls", "access", path, e);
            } else {
                diag::cannot("ls", "open directory", path, e);
            }
            return Err(());
        }
        Err(ReadDirError::Read(e)) => {
            diag::report("ls", "reading directory", path, e);
            return Err(());
        }
    };
    let mut items: Vec<Item> = entries
        .into_iter()
        .filter(|ent| o.all || !ent.name.starts_with('.'))
        .map(|ent| Item { name: ent.name, size: ent.size, attrs: ent.attrs, time: 0 })
        .collect();
    if o.by_time && !o.by_size {
        for item in &mut items {
            if let Ok(info) = stat(&join(path, &item.name)) {
                item.time = time_key(&info);
            }
        }
    }
    sort(&mut items, o);
    for item in &items {
        print(item, o);
    }

    let mut result = Ok(());
    if o.recursive {
        for item in items.iter().filter(|item| item.is_dir()) {
            let child = join(path, &item.name);
            let _ = writeln!(Fd(1));
            let _ = writeln!(Fd(1), "{child}:");
            if list_dir(&child, o).is_err() {
                result = Err(());
            }
        }
    }
    result
}

fn run(args: userlib::Args) -> ExitCode {
    cli::status(list(args))
}

fn list(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut o = Options::default();
    let mut n_operands = 0usize;

    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("ls", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('a') | Arg::Long("all") => o.all = true,
            Arg::Short('d') | Arg::Long("directory") => o.dir_only = true,
            Arg::Short('F') | Arg::Long("classify") => o.classify = true,
            Arg::Short('h') | Arg::Long("human-readable") => o.human = true,
            Arg::Short('l') => o.long = true,
            Arg::Short('R') | Arg::Long("recursive") => o.recursive = true,
            Arg::Short('r') | Arg::Long("reverse") => o.reverse = true,
            Arg::Short('S') => o.by_size = true,
            Arg::Short('t') => o.by_time = true,
            // One entry per line is all this `ls` ever does, so `-1` is accepted and changes nothing.
            Arg::Short('1') => {}
            Arg::Positional(_) => n_operands += 1,
            other => return Err(cli::invalid("ls", other)),
        }
    }

    let mut operands: Vec<&'static str> = cli::operands(args).collect();
    if operands.is_empty() {
        operands.push(".");
    }

    // Each operand is a file (listed as itself, all of them first) or a directory (its contents, after them).
    let mut status = 0;
    let mut files: Vec<Item> = Vec::new();
    let mut dirs: Vec<&str> = Vec::new();
    for &operand in &operands {
        match stat(operand) {
            Ok(info) if info.attrs & ATTR_DIRECTORY != 0 && !o.dir_only => dirs.push(operand),
            Ok(info) => files.push(Item {
                name: String::from(operand),
                size: info.size,
                attrs: info.attrs,
                time: time_key(&info),
            }),
            Err(e) => {
                diag::cannot("ls", "access", operand, e);
                status = 1;
            }
        }
    }

    sort(&mut files, &o);
    for item in &files {
        print(item, &o);
    }

    // A header names each directory when there is more than one thing to list, or when recursing.
    let headers = n_operands > 1 || o.recursive;
    // Directories come in the order sorted by name (or as `-t`/`-S`/`-r` say), as GNU does for operands.
    dirs.sort();
    if o.reverse {
        dirs.reverse();
    }
    for (i, dir) in dirs.iter().enumerate() {
        if i > 0 || !files.is_empty() {
            let _ = writeln!(Fd(1));
        }
        if headers {
            let _ = writeln!(Fd(1), "{dir}:");
        }
        if list_dir(dir, &o).is_err() {
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
