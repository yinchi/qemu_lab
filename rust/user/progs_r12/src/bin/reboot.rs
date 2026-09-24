//! `reboot` -- restarts the machine via the kernel's `reboot` syscall (PSCI underneath); equivalent
//! to `poweroff --reboot` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{diag, fail};
use userlib::{ExitCode, LINUX_REBOOT_CMD_RESTART, reboot};
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "reboot";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("reboot", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if let Some(operand) = plain.first {
        return diag::extra_operand("reboot", operand);
    }
    // Only reachable if the kernel somehow rejected the one command this program ever passes.
    let err = reboot(LINUX_REBOOT_CMD_RESTART);
    fail("reboot", "cannot restart", err);
    ExitCode(1)
}
