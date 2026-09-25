//! `stat FILE...` -- see `docs/progs.md`.
//!
//! Prints every field FAT actually stores for each operand: size, type, the two tracked attribute
//! bits, and all three real timestamps (`userlib::stat`'s raw, packed fields -- see its doc comment).
//!
//! FAT keeps no time zone and this system stores **UTC** (Stage 14), so a stamp is converted to the local
//! zone (`progs_r15::LOCAL_ZONE`, America/Toronto) for display, with the zone's abbreviation, as `date`
//! shows it: `2001-09-08 21:46:40 EDT`.
//!
//! The accessed date is not shown. FAT stores only a date for it, nothing here updates it on a read (Linux's
//! `noatime`), and it is set only when an entry is created or written -- to the modified date -- so it would
//! always repeat the line above it, in UTC beside a local time that can fall on another day. The syscall still
//! returns it (`userlib::Stat::accessed_date`), for a later `relatime`-style update or an `ls -lu`.

#![no_std]
#![no_main]

use core::fmt::Write;

use chrono::{Datelike, NaiveDate, Timelike};
use chrono_tz::OffsetName;
use progs::{Fd, diag};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, ExitCode, Stat, stat};
use progs_r12::cli;
use progs_r15::LOCAL_ZONE;

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

/// `Label: YYYY-MM-DD HH:MM:SS ZONE`, the packed UTC stamp converted to the local zone. A stamp that is not a
/// real date (FAT stores whatever it is given) is shown as stored.
fn write_stamp(out: &mut Fd, label: &str, date: u16, time: u16) {
    let (y, mo, d) = split_date(date);
    let (h, mi, s) = split_time(time);
    let utc = NaiveDate::from_ymd_opt(i32::from(y), u32::from(mo), u32::from(d))
        .and_then(|day| day.and_hms_opt(u32::from(h), u32::from(mi), u32::from(s)));
    let Some(utc) = utc else {
        let _ = writeln!(out, "{label}: {y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} (not a valid date)");
        return;
    };
    let local = utc.and_utc().with_timezone(&LOCAL_ZONE);
    let _ = writeln!(
        out,
        "{label}: {:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
        local.year(),
        local.month(),
        local.day(),
        local.hour(),
        local.minute(),
        local.second(),
        local.offset().abbreviation().unwrap_or("?"),
    );
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
    write_stamp(&mut out, "Modify", info.modified_date, info.modified_time);
    write_stamp(&mut out, "Create", info.created_date, info.created_time);
}

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("stat", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        return diag::missing_operand("stat");
    }

    let mut status = 0;
    for (i, arg) in cli::operands(args).enumerate() {
        if i > 0 {
            let _ = writeln!(Fd(1));
        }
        match stat(arg) {
            Ok(info) => print_one(arg, &info),
            Err(e) => {
                diag::cannot("stat", "stat", arg, e);
                status = 1;
            }
        }
    }
    ExitCode(status)
}
