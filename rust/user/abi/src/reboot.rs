//! `reboot(2)`'s `cmd` values this project implements. Linux defines several more
//! (`LINUX_REBOOT_CMD_HALT`, `_CAD_ON`/`_OFF`, `_RESTART2`, `_KEXEC`, `_SW_SUSPEND`), none of which
//! map to a capability this kernel has (no ACPI, no kexec, no way to halt without powering off on
//! `virt`) -- `SYS_REBOOT` returns `EINVAL` for any of them, the same as an unrecognized value does
//! on real Linux.

/// Powers off the machine.
pub const LINUX_REBOOT_CMD_POWER_OFF: u32 = 0x4321_FEDC;
/// Restarts the machine.
pub const LINUX_REBOOT_CMD_RESTART: u32 = 0x0123_4567;

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux's real values.
    #[test]
    fn values_are_linuxs_numbers() {
        assert_eq!(
            [LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART],
            [0x4321_FEDC, 0x0123_4567]
        );
    }
}
