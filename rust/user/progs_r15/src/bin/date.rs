//! `date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]` -- see `docs/progs.md`. Prints the time from the
//! real-time clock (or the time `-d @SECONDS` names) in the local time zone, `America/Toronto`, or in UTC
//! with `-u`.
//!
//! The calendar arithmetic, the time-zone rules (offsets, daylight saving, the abbreviations `EST` and
//! `EDT`) and the `strftime` conversions are `chrono`'s and `chrono-tz`'s -- the whole IANA database is in
//! this binary, which is why Stage 15's larger program window exists.

#![no_std]
#![no_main]

use core::fmt::{Display, Write};

use chrono::{DateTime, TimeZone, Utc};
use getargs::Arg;
use linked_list_allocator::LockedHeap;
use progs::{Fd, diag, fail, help};
use progs_r12::cli;
use progs_r15::LOCAL_ZONE;
use userlib::{ExitCode, time};

userlib::entry_with_args!(run);

/// `chrono` formats through `alloc`, and EL0 has no heap until Stage 16, so `date` brings a small fixed
/// one: a static array handed to `linked_list_allocator` at start-up. Stage 16's `userlib` heap replaces it.
#[global_allocator]
static HEAP: LockedHeap = LockedHeap::empty();

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP_SPACE: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

const USAGE: &str = "date [-u] [-d @SECONDS] [-I[FMT] | -R | +FORMAT]";
const FLAGS: &[(&str, &str)] = &[
    ("-u", "print UTC instead of the local time (America/Toronto)"),
    ("-d @N", "show the time N seconds after 1970-01-01 00:00:00 UTC instead of now"),
    ("-I[FMT]", "ISO 8601: FMT is date (the default), hours, minutes or seconds"),
    ("-R", "RFC 5322 format"),
    ("+FORMAT", "strftime-style format (%Y %m %d %H %M %S %s %Z %z %a %A %b %B %e %j %F %T ... %%)"),
];

/// GNU's default layout in the C locale.
const DEFAULT_FORMAT: &str = "%a %b %e %H:%M:%S %Z %Y";
const RFC_EMAIL_FORMAT: &str = "%a, %d %b %Y %H:%M:%S %z";

/// What to print, and at what time.
struct Request {
    format: &'static str,
    /// `-d @N`; `None` is now.
    at: Option<i64>,
    /// `-u`: UTC, not the local zone.
    utc: bool,
}

/// Reads the arguments. `Err` is the status to return (`--help`, or a problem already reported).
fn parse(args: userlib::Args) -> Result<Request, ExitCode> {
    let mut format: Option<&'static str> = None;
    let mut at = None;
    let mut utc = false;
    let mut opts = cli::opts(args);

    while let Some(arg) = cli::next("date", &mut opts)? {
        let chosen = match arg {
            Arg::Long("help") => return Err(help(USAGE, FLAGS)),
            Arg::Short('u') | Arg::Long("utc" | "universal") => {
                utc = true;
                continue;
            }
            Arg::Short('d') | Arg::Long("date") => {
                let text = cli::value("date", &mut opts)?;
                at = Some(parse_at(text)?);
                continue;
            }
            Arg::Short('R') | Arg::Long("rfc-email") => RFC_EMAIL_FORMAT,
            Arg::Short('I') | Arg::Long("iso-8601") => iso_format(opts.value_opt())?,
            Arg::Positional(text) => match text.strip_prefix('+') {
                Some(spec) => spec,
                None => return Err(invalid_date(text)),
            },
            other => return Err(cli::invalid("date", other)),
        };
        if format.replace(chosen).is_some() {
            let _ = writeln!(Fd(2), "date: multiple output formats specified");
            return Err(ExitCode(1));
        }
    }
    Ok(Request { format: format.unwrap_or(DEFAULT_FORMAT), at, utc })
}

fn invalid_date(text: &str) -> ExitCode {
    let _ = writeln!(Fd(2), "date: invalid date '{text}'");
    ExitCode(1)
}

/// `-d`'s operand: `@N`, seconds since the epoch. (GNU also reads free-form dates and relative times;
/// there is no such parser here.)
fn parse_at(text: &str) -> Result<i64, ExitCode> {
    text.strip_prefix('@')
        .and_then(|digits| digits.parse::<i64>().ok())
        .ok_or_else(|| invalid_date(text))
}

/// The layout for `-I[FMT]`. The offset is the real one (`-04:00`), not always `+00:00`.
fn iso_format(precision: Option<&str>) -> Result<&'static str, ExitCode> {
    match precision {
        None | Some("date") => Ok("%Y-%m-%d"),
        Some("hours") => Ok("%Y-%m-%dT%H%:z"),
        Some("minutes") => Ok("%Y-%m-%dT%H:%M%:z"),
        Some("seconds") => Ok("%Y-%m-%dT%H:%M:%S%:z"),
        Some(other) => {
            let _ = writeln!(Fd(2), "date: invalid argument '{other}' for '--iso-8601'");
            diag::try_help("date");
            Err(ExitCode(1))
        }
    }
}

/// Prints `instant` in `format` and a newline. A conversion `chrono` does not know makes its `Display` fail;
/// nothing has been written by then.
fn print<Z: TimeZone>(instant: &DateTime<Z>, format: &str) -> ExitCode
where
    Z::Offset: Display,
{
    if write!(Fd(1), "{}", instant.format(format)).is_err() {
        let _ = writeln!(Fd(2), "date: invalid format '+{format}'");
        return ExitCode(1);
    }
    let _ = writeln!(Fd(1));
    ExitCode(0)
}

fn run(args: userlib::Args) -> ExitCode {
    // SAFETY: `HEAP_SPACE` is used only here, once, before anything allocates.
    unsafe { HEAP.lock().init((&raw mut HEAP_SPACE).cast::<u8>(), HEAP_SIZE) };
    let request = match parse(args) {
        Ok(request) => request,
        Err(status) => return status,
    };
    let unix = match request.at {
        Some(unix) => unix,
        None => match time() {
            Ok(unix) => unix,
            Err(e) => {
                fail("date", "cannot read the clock", e);
                return ExitCode(1);
            }
        },
    };
    let Some(instant) = DateTime::<Utc>::from_timestamp(unix, 0) else {
        let _ = writeln!(Fd(2), "date: time out of range");
        return ExitCode(1);
    };
    if request.utc {
        print(&instant, request.format)
    } else {
        print(&instant.with_timezone(&LOCAL_ZONE), request.format)
    }
}
