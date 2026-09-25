//! What the Stage 15 programs share: the local time zone.

#![no_std]

use chrono_tz::Tz;

/// The local time zone, for the programs that show a time (`date`, `stat`). There is no `$TZ` until Stage 17
/// (environment variables), which replaces this with the variable's value -- `chrono_tz` parses any IANA
/// name -- and UTC when it is unset.
pub const LOCAL_ZONE: Tz = chrono_tz::America::Toronto;
