//! Unix time to the calendar fields a FAT directory entry stores. Pure logic, on `chrono` (no clock, no
//! time zones): tested on the host (`hosttests/`). `rtc_time.rs` turns the fields into hadris-fat's
//! `FatDateTime` and supplies them from the real-time clock.
//!
//! Timestamps are stored in **UTC**. FAT has no time-zone field and Windows reads the fields as local time,
//! but the kernel never interprets them: it writes what the clock says (UTC) and `stat` hands them back
//! raw, so read and write agree, as they do on Linux with `mount -o tz=UTC`. Converting to a user's zone
//! is a display matter for the program that prints them (Stage 17's `$TZ`).

use chrono::{DateTime, Datelike, Timelike, Utc};

/// The calendar fields of one FAT timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    /// 1980 to 2107, the range FAT can store.
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// 0 to 59. FAT stores 2-second steps, so an odd second is rounded down on disk.
    pub second: u8,
}

/// 1980-01-01 00:00:00, the FAT epoch and the earliest time it can hold.
pub const FAT_EPOCH: Fields = Fields { year: 1980, month: 1, day: 1, hour: 0, minute: 0, second: 0 };

/// The last time FAT can hold (its date runs to 2107-12-31; its time to 23:59:58).
const FAT_LAST: Fields = Fields { year: 2107, month: 12, day: 31, hour: 23, minute: 59, second: 58 };

/// The fields for `unix` seconds since 1970. A time before 1980 or after 2107 is clamped whole to the
/// nearest end of what FAT can hold (a 1970 clock, an unset RTC, becomes the FAT epoch, not "1980 with
/// 1970's month and day").
pub fn fields(unix: i64) -> Fields {
    let Some(t) = DateTime::<Utc>::from_timestamp(unix, 0) else {
        return if unix < 0 { FAT_EPOCH } else { FAT_LAST };
    };
    if t.year() < 1980 {
        return FAT_EPOCH;
    }
    if t.year() > 2107 {
        return FAT_LAST;
    }
    Fields {
        year: t.year() as u16,
        month: t.month() as u8,
        day: t.day() as u8,
        hour: t.hour() as u8,
        minute: t.minute() as u8,
        second: t.second() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Fields {
        Fields { year, month, day, hour, minute, second }
    }

    #[test]
    fn a_time_in_range_is_broken_down_as_utc() {
        assert_eq!(fields(1_000_000_000), at(2001, 9, 9, 1, 46, 40));
        assert_eq!(fields(1_709_164_800), at(2024, 2, 29, 0, 0, 0)); // a leap day
        assert_eq!(fields(4_107_542_400), at(2100, 3, 1, 0, 0, 0)); // 2100 is not a leap year
    }

    #[test]
    fn the_first_and_last_moments_fat_can_hold() {
        assert_eq!(fields(315_532_800), FAT_EPOCH); // 1980-01-01 00:00:00 exactly
        assert_eq!(fields(315_532_799), FAT_EPOCH); // one second earlier clamps up to it
        assert_eq!(fields(4_354_819_199), at(2107, 12, 31, 23, 59, 59)); // still in range: FAT drops the odd second
        assert_eq!(fields(4_354_819_200), FAT_LAST); // 2108-01-01 clamps down
    }

    #[test]
    fn an_unset_clock_is_the_fat_epoch_whole() {
        assert_eq!(fields(0), FAT_EPOCH);
        assert_eq!(fields(15_000_000), FAT_EPOCH); // 1970-06-25: the whole date clamps, not just the year
        assert_eq!(fields(-1), FAT_EPOCH);
        assert_eq!(fields(i64::MIN), FAT_EPOCH);
    }

    #[test]
    fn a_time_too_large_to_represent_is_the_last_fat_moment() {
        assert_eq!(fields(i64::MAX), FAT_LAST);
    }

    #[test]
    fn the_rtcs_whole_range_maps_without_a_panic() {
        // Every 100000th second across the PL031's 32-bit range.
        for unix in (0..=u32::MAX as i64).step_by(100_000) {
            let f = fields(unix);
            assert!((1980..=2107).contains(&f.year), "{unix}");
            assert!((1..=12).contains(&f.month) && (1..=31).contains(&f.day), "{unix}");
        }
    }
}
