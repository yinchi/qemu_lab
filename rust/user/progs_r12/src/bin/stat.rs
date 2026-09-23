//! `stat FILE...` -- see `docs/progs.md`.
//!
//! Prints every field FAT actually stores for each operand: size, type, the two tracked attribute
//! bits, and all three real timestamps (`userlib::stat`'s raw, packed fields -- see its doc
//! comment). No RTC exists yet (`Stage12.md`'s Step 10 section), so every timestamp reads as
//! either the fixed build-time stamp bundled files got, or the FAT epoch for anything the kernel
//! itself created or wrote during the running session -- both real stored values, not
//! placeholders.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, fail, help, unknown_option};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, ExitCode, Stat, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "stat FILE...";
const FLAGS: &[(&str, &str)] = &[];

/// One FAT packed date (bits 0-4 day, 5-8 month, 9-15 year-since-1980) as `(year, month, day)`.
fn split_date(date: u16) -> (u16, u16, u16) {
    (1980 + (date >> 9), (date >> 5) & 0x0f, date & 0x1f)
}

/// One FAT packed time (bits 0-4 seconds/2, 5-10 minutes, 11-15 hours) as `(hour, minute, second)`.
fn split_time(time: u16) -> (u16, u16, u16) {
    (time >> 11, (time >> 5) & 0x3f, (time & 0x1f) * 2)
}

fn print_one(name: &str, info: &Stat) {
    let mut out = Fd(1);
    let kind = if info.attrs & ATTR_DIRECTORY != 0 { "directory" } else { "regular file" };
    let _ = writeln!(out, "  File: {name}");
    let _ = writeln!(out, "  Size: {:<12} Type: {kind}", info.size);
    let _ = writeln!(
        out,
        " Attrs: read-only={}  exec={}",
        if info.attrs & ATTR_READ_ONLY != 0 { "yes" } else { "no" },
        if info.attrs & ATTR_EXEC != 0 { "yes" } else { "no" }
    );
    let (y, mo, d) = split_date(info.modified_date);
    let (h, mi, s) = split_time(info.modified_time);
    let _ = writeln!(out, "Modify: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
    let (y, mo, d) = split_date(info.created_date);
    let (h, mi, s) = split_time(info.created_time);
    let _ = writeln!(out, "Create: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}");
    let (y, mo, d) = split_date(info.accessed_date);
    let _ = writeln!(out, "Access: {y:04}-{mo:02}-{d:02}");
}

fn run(args: userlib::Args) -> ExitCode {
    let mut any = false;
    let mut status = 0;
    let mut first = true;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("stat", arg);
        }
        any = true;
        if !first {
            let _ = writeln!(Fd(1));
        }
        first = false;
        match stat(arg) {
            Ok(info) => print_one(arg, &info),
            Err(e) => {
                fail("stat", arg, e);
                status = 1;
            }
        }
    }

    if !any {
        return progs::usage(USAGE);
    }

    ExitCode(status)
}
