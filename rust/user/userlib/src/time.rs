//! The clock: `clock_gettime` and the `time` convenience over it. The kernel has one clock, the
//! real-time clock, in whole seconds since the Unix epoch, UTC (see `abi::time`).

use abi::syscall::SYS_CLOCK_GETTIME;
use abi::time::{CLOCK_REALTIME, TIMESPEC_SIZE};

/// Reads `clock` (only `CLOCK_REALTIME` exists): `(seconds, nanoseconds)`, or a negative error --
/// `EINVAL` for any other clock.
pub fn clock_gettime(clock: usize) -> Result<(i64, i64), isize> {
    let mut out = [0u8; TIMESPEC_SIZE];
    let result = syscall!(SYS_CLOCK_GETTIME, clock, out.as_mut_ptr() as usize);
    if result < 0 {
        return Err(result);
    }
    let sec = i64::from_le_bytes(out[0..8].try_into().unwrap());
    let nsec = i64::from_le_bytes(out[8..16].try_into().unwrap());
    Ok((sec, nsec))
}

/// The current time in whole seconds since 1970-01-01 00:00:00 UTC.
pub fn time() -> Result<i64, isize> {
    clock_gettime(CLOCK_REALTIME).map(|(sec, _)| sec)
}
