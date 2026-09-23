//! `chmod +x|-x|+w|-w [-R] file...` -- see `docs/progs.md`. Stage 12's tier replaces the base
//! `chmod` (one file only, and hand-rolled its "invalid mode" message inconsistently with every
//! other program's error wording) with multi-file and `-R` support -- `-R` needs the `stat`
//! syscall this stage adds, which r09-r11's older kernels don't have.

#![no_std]
#![no_main]

use core::fmt::Write;

use abi::errno::ENAMETOOLONG;
use progs::{Fd, PathBuf, fail, help, unknown_option, usage};
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

fn run(args: userlib::Args) -> ExitCode {
    let mut args = args.skip(1);

    let Some(mode) = args.next() else {
        return usage(USAGE);
    };
    if mode == "--help" {
        return help(USAGE, FLAGS);
    }

    // FAT has no write bit, only a read-only bit, so `+w` clears it and `-w` sets it.
    let (set, clear) = match mode {
        "+x" => (ATTR_EXEC, 0),
        "-x" => (0, ATTR_EXEC),
        "+w" => (0, ATTR_READ_ONLY),
        "-w" => (ATTR_READ_ONLY, 0),
        _ => {
            let _ = writeln!(Fd(2), "chmod: {mode}: invalid mode");
            return ExitCode(1);
        }
    };

    let mut recursive = false;
    let mut any = false;
    let mut status = 0;

    for path in args {
        if path == "-R" {
            recursive = true;
            continue;
        }
        if path.len() > 1 && path.starts_with('-') {
            return unknown_option("chmod", path);
        }
        any = true;

        let mut ok = true;
        let r = chmod(path, set, clear);
        if r < 0 {
            fail("chmod", path, r);
            ok = false;
        }

        if recursive && ok {
            match stat(path) {
                Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
                    if let Err(e) = chmod_recursive(path, set, clear) {
                        fail("chmod", path, e);
                        ok = false;
                    }
                }
                Ok(_) => {} // a plain file: -R beyond the chmod already done is a no-op
                Err(e) => {
                    fail("chmod", path, e);
                    ok = false;
                }
            }
        }

        if !ok {
            status = 1;
        }
    }

    if !any {
        return usage(USAGE);
    }

    ExitCode(status)
}
