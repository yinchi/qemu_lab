//! The PL031 real-time clock: a count of seconds since the Unix epoch (UTC) that QEMU's `virt` machine
//! sets from the host's clock at start-up and keeps advancing.
//!
//! Only one register is needed: `RTCDR` (offset 0), the data register, a read-only 32-bit count. The
//! load, match and interrupt registers exist to set the clock or raise an alarm, and nothing here does
//! either. A 32-bit count of seconds runs out in 2106.

use core::ptr::read_volatile;

use super::base_addresses::RTC_BASE;

/// `RTCDR`: the current time, in seconds since 1970-01-01 00:00:00 UTC.
const RTCDR: usize = RTC_BASE;

/// The current time as whole seconds since the Unix epoch, read from the hardware.
pub fn seconds() -> u32 {
    // SAFETY: `RTC_BASE` is mapped as device memory by `arch/mmu.rs` before any caller runs, and a read
    // of `RTCDR` has no side effect.
    unsafe { read_volatile(RTCDR as *const u32) }
}
