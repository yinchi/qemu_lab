//! The clock hadris-fat stamps directory entries with: the real-time clock, in UTC (see `fattime.rs`).
//!
//! The volume takes a `&'static dyn TimeProvider` when it is mounted (`main.rs`) and asks it for "now"
//! whenever an entry is created, written or renamed. Without one it stamps every entry with the FAT
//! epoch, which is what Stages 8-13 did.

use hadris_fat::time::{FatDateTime, TimeProvider};

use super::fattime::fields;
use crate::platform::rtc;

/// Stamps from the PL031 real-time clock.
#[derive(Debug)]
pub struct RtcTimeProvider;

/// The one provider, handed to the volume at mount (it needs a `'static` reference).
pub static RTC_TIME: RtcTimeProvider = RtcTimeProvider;

impl TimeProvider for RtcTimeProvider {
    fn now(&self) -> FatDateTime {
        let unix = i64::from(rtc::seconds());
        let f = fields(unix);
        let mut stamp = FatDateTime::new(f.year, f.month, f.day, f.hour, f.minute, f.second);
        // The creation time has a 10 ms field that carries the odd second FAT's 2-second steps drop.
        stamp.time_tenth = (f.second % 2) * 100;
        stamp
    }
}
