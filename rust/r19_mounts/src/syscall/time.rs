//! The `clock_gettime` syscall: the time from the real-time clock (`platform::rtc`).

use abi::errno::{EFAULT, EINVAL};
use abi::time::{CLOCK_REALTIME, TIMESPEC_SIZE};

use super::fd::validate;
use crate::platform::rtc;

/// Writes the current time, as a `timespec` (`tv_sec: i64`, `tv_nsec: i64`, little-endian), to the user
/// buffer `ptr`. Only `CLOCK_REALTIME` exists (`EINVAL` for any other clock), and the clock counts whole
/// seconds, so `tv_nsec` is 0. `EFAULT` if `ptr` is not writable user memory.
pub fn clock_gettime(clock: usize, ptr: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // writes a user buffer: clear PAN while it does
    if clock != CLOCK_REALTIME {
        return EINVAL;
    }
    if !validate(ptr, TIMESPEC_SIZE, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let out = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, TIMESPEC_SIZE) };
    out[0..8].copy_from_slice(&i64::from(rtc::seconds()).to_le_bytes());
    out[8..16].copy_from_slice(&0i64.to_le_bytes());
    0
}
