//! `clock_gettime(2)`'s clock ids and `struct timespec`, as this project implements them: one clock,
//! the real-time clock's Unix time in UTC.
//!
//! `clock_gettime` fills a 16-byte `timespec` at the user pointer: `tv_sec: i64` then `tv_nsec: i64`,
//! both little-endian, as on Linux aarch64. The PL031 counts whole seconds, so `tv_nsec` is always 0.

/// `CLOCK_REALTIME`: wall-clock time, seconds since 1970-01-01 00:00:00 UTC. The only clock there is;
/// any other id is `EINVAL`. (`CLOCK_MONOTONIC` would want the generic timer, Stage 23's business.)
pub const CLOCK_REALTIME: usize = 0;

/// Size of the `timespec` `clock_gettime` fills in.
pub const TIMESPEC_SIZE: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux's real values.
    #[test]
    fn values_are_linuxs() {
        assert_eq!((CLOCK_REALTIME, TIMESPEC_SIZE), (0, 16));
    }
}
