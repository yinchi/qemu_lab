//! `ls [-F] [-l] [dir...]` -- see `docs/progs.md`. Stage 12's tier replaces the base `ls` (same
//! options and output) in one respect: with no operand it lists the working directory, not the
//! root -- the base tier's programs predate the working directory, and r09-r11 still use them.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, fail, help, unknown_option};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, close, getdents, open};

userlib::entry_with_args!(run);

const USAGE: &str = "ls [-F] [-l] [dir...]";
const FLAGS: &[(&str, &str)] = &[
    ("-F", "append / to directories and * to executable files"),
    ("-l", "long format: d/w/x flags, size, name"),
];

/// How many records to fetch per `getdents` call.
const BATCH: usize = 8;

/// Lists one directory. Returns `Err(())` (already reported) on failure.
fn list_one(dir: &str, classify: bool, long: bool) -> Result<(), ()> {
    let fd = open(dir, O_RDONLY);
    if fd < 0 {
        fail("ls", dir, fd);
        return Err(());
    }
    let fd = fd as usize;

    let mut buf = [0u8; DIRENT_SIZE * BATCH];
    let mut ok = true;

    loop {
        let n = getdents(fd, &mut buf);
        if n < 0 {
            fail("ls", dir, n);
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }

        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            let Some(ent) = DirEnt::parse(raw) else { continue };

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
    }
    close(fd);
    if ok { Ok(()) } else { Err(()) }
}

fn run(args: userlib::Args) -> ExitCode {
    let mut classify = false;
    let mut long = false;
    let mut n_dirs = 0usize;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        } else if arg == "-F" {
            classify = true;
        } else if arg == "-l" {
            long = true;
        } else if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("ls", arg);
        } else {
            n_dirs += 1;
        }
    }

    let mut status = 0;

    if n_dirs == 0 {
        if list_one(".", classify, long).is_err() {
            status = 1;
        }
        return ExitCode(status);
    }

    let mut first = true;
    for arg in args.skip(1) {
        if arg.len() > 1 && arg.starts_with('-') {
            continue; // already validated as a flag above
        }
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

    ExitCode(status)
}
