//! `reboot` -- restarts the machine via the kernel's `reboot` syscall (PSCI underneath); equivalent
//! to `poweroff --reboot` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{diag, fail, help};
use userlib::{ExitCode, LINUX_REBOOT_CMD_RESTART, reboot};

userlib::entry_with_args!(run);

const USAGE: &str = "reboot";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: userlib::Args) -> ExitCode {
    if let Some(arg) = args.nth(1) {
        return match arg {
            "--help" => help(USAGE, FLAGS),
            _ if arg.starts_with('-') => diag::invalid_option("reboot", arg),
            _ => diag::extra_operand("reboot", arg),
        };
    }
    // Only reachable if the kernel somehow rejected the one command this program ever passes.
    let err = reboot(LINUX_REBOOT_CMD_RESTART);
    fail("reboot", "cannot restart", err);
    ExitCode(1)
}
