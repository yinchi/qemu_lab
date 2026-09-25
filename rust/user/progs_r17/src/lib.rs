//! What the Stage 17 programs share: the local time zone, from the environment.

#![no_std]

use chrono_tz::Tz;

/// The local time zone for the programs that show a time (`date`, `stat`): the IANA name in `$TZ`
/// (`America/Toronto`, `Europe/London`, `UTC`, ...; a leading `:` as POSIX allows is ignored), and UTC when
/// `TZ` is unset, empty, or not a name in the database -- glibc's behaviour too. POSIX rule strings
/// (`EST5EDT,M3.2.0,M11.1.0`) are not understood, so they mean UTC.
///
/// A program must be started with `userlib::entry_with_env!`, or it sees no environment and this is UTC.
pub fn local_zone() -> Tz {
    userlib::env::var("TZ")
        .map(|name| name.strip_prefix(':').unwrap_or(name))
        .and_then(|name| name.parse::<Tz>().ok())
        .unwrap_or(Tz::UTC)
}
