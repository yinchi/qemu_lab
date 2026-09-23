//! The `reboot` syscall: powers off or restarts the machine via PSCI (`arch::psci`).

use abi::errno::EINVAL;
use abi::reboot::{LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};

use crate::arch::psci;

/// Powers off (`LINUX_REBOOT_CMD_POWER_OFF`) or restarts (`LINUX_REBOOT_CMD_RESTART`) the machine;
/// never returns on success. Any other `cmd` returns `EINVAL`, matching what real Linux does for an
/// unrecognized `reboot(2)` command.
pub fn reboot(cmd: u32) -> isize {
    match cmd {
        LINUX_REBOOT_CMD_POWER_OFF => psci::system_off(),
        LINUX_REBOOT_CMD_RESTART => psci::system_reset(),
        _ => EINVAL,
    }
}
