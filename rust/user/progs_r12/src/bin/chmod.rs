//! `chmod +x|-x|+w|-w [-R] file...` -- see `docs/progs.md`. Stage 12's tier replaces the base
//! `chmod` (one file only) with multi-file and `-R` support -- `-R` needs the `stat` syscall this stage
//! adds, which r09-r11's older kernels don't have. Only those four symbolic modes exist (a single
//! user, and FAT has just an executable bit and a read-only bit), so anything else, an octal `755`
//! included, is an invalid mode.

#![no_std]
#![no_main]

use core::fmt::Write;

use abi::errno::{ENAMETOOLONG, ENOENT};
use progs::{Fd, PathBuf, diag, help};
use userlib::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, chmod,
    close, getdents, open, stat,
};

userlib::entry_with_args!(run);

const USAGE: &str = "chmod +x|-x|+w|-w [-R] file...";
const FLAGS: &[(&str, &str)] = &[("-R", "recurse into directory operands")];

/// How many `getdents` records are pulled into the stack buffer per batch (matches `ls`'s own
/// `BATCH`).
const BATCH: usize = 8;

/// Applies `set`/`clear` to every descendant of the directory `path`, depth-first, files and
/// subdirectories alike -- `path` itself is not touched (the caller already did that). Unlike
/// `rm -r`, nothing here shrinks the directory as it goes, so this reads each directory's whole
/// listing (batched `getdents`, like `ls`) rather than reopening one entry at a time.
fn chmod_recursive(path: &str, set: u8, clear: u8) -> Result<(), isize> {
    let fd = open(path, O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let fd = fd as usize;
    let mut buf = [0u8; DIRENT_SIZE * BATCH];
    let mut first_err: Option<isize> = None;

    loop {
        let n = getdents(fd, &mut buf);
        if n < 0 {
            close(fd);
            return Err(n);
        }
        if n == 0 {
            break;
        }
        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            let Some(ent) = DirEnt::parse(raw) else { continue };
            let Some(child) = PathBuf::join(path, ent.name) else {
                first_err.get_or_insert(ENAMETOOLONG);
                continue;
            };
            if ent.attrs & ATTR_DIRECTORY != 0
                && let Err(e) = chmod_recursive(child.as_str(), set, clear)
            {
                first_err.get_or_insert(e);
            }
            let r = chmod(child.as_str(), set, clear);
            if r < 0 {
                first_err.get_or_insert(r);
            }
        }
    }
    close(fd);
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// GNU's wording: a name that does not exist is "cannot access", any other failure "changing permissions of".
fn report(path: &str, code: isize) {
    if code == ENOENT {
        diag::cannot("chmod", "access", path, code);
    } else {
        diag::report("chmod", "changing permissions of", path, code);
    }
}

/// The attribute bits a mode changes -- `(set, clear)` -- for one of the four symbolic modes.
/// FAT has no write bit, only a read-only bit, so `+w` clears it and `-w` sets it.
fn mode_bits(mode: &str) -> Option<(u8, u8)> {
    match mode {
        "+x" => Some((ATTR_EXEC, 0)),
        "-x" => Some((0, ATTR_EXEC)),
        "+w" => Some((0, ATTR_READ_ONLY)),
        "-w" => Some((ATTR_READ_ONLY, 0)),
        _ => None,
    }
}

/// What one command-line argument is.
enum Item {
    Help,
    Recursive,
    /// The first operand.
    Mode(&'static str),
    File(&'static str),
    /// Looks like an option this program does not have.
    Bad(&'static str),
}

/// The arguments in order, classified. `chmod` cannot use `getargs` for this: its modes `-x` and `-w`
/// look like options, so an argument is an option only if it is `-R`, `--help` or `--`, and a
/// dash-word that is not one of the four modes is unrecognized. Options may come anywhere, as in GNU.
struct Items {
    args: core::iter::Skip<userlib::Args>,
    ended: bool,
    mode_seen: bool,
}

impl Items {
    fn new(args: userlib::Args) -> Self {
        Items { args: args.skip(1), ended: false, mode_seen: false }
    }
}

impl Iterator for Items {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        loop {
            let arg = self.args.next()?;
            if !self.ended {
                if arg == "--" {
                    self.ended = true;
                    continue;
                }
                if arg == "--help" {
                    return Some(Item::Help);
                }
                if arg == "-R" {
                    return Some(Item::Recursive);
                }
                if arg.len() > 1 && arg.starts_with('-') && mode_bits(arg).is_none() {
                    return Some(Item::Bad(arg));
                }
            }
            if !self.mode_seen {
                self.mode_seen = true;
                return Some(Item::Mode(arg));
            }
            return Some(Item::File(arg));
        }
    }
}

fn run(args: userlib::Args) -> ExitCode {
    let mut mode = None;
    let mut bits = (0, 0);
    let mut recursive = false;
    let mut files = 0usize;

    // First pass: the mode and the flags, so that `-R` applies to every file wherever it is written.
    for item in Items::new(args) {
        match item {
            Item::Help => return help(USAGE, FLAGS),
            Item::Recursive => recursive = true,
            Item::Bad(arg) => return diag::invalid_option("chmod", arg),
            Item::Mode(text) => {
                let Some(found) = mode_bits(text) else {
                    let _ = writeln!(Fd(2), "chmod: invalid mode: '{text}'");
                    return ExitCode(1);
                };
                mode = Some(text);
                bits = found;
            }
            Item::File(_) => files += 1,
        }
    }
    let Some(mode) = mode else {
        return diag::missing_operand("chmod");
    };
    if files == 0 {
        return diag::missing_operand_after("chmod", mode);
    }
    let (set, clear) = bits;

    let mut status = 0;
    for item in Items::new(args) {
        let Item::File(path) = item else { continue };

        let mut ok = true;
        let r = chmod(path, set, clear);
        if r < 0 {
            report(path, r);
            ok = false;
        }

        if recursive && ok {
            match stat(path) {
                Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
                    if let Err(e) = chmod_recursive(path, set, clear) {
                        report(path, e);
                        ok = false;
                    }
                }
                Ok(_) => {} // a plain file: -R beyond the chmod already done is a no-op
                Err(e) => {
                    report(path, e);
                    ok = false;
                }
            }
        }

        if !ok {
            status = 1;
        }
    }

    ExitCode(status)
}
