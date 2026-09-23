//! `poweroff [--reboot]` -- powers off the machine, or restarts it with `--reboot`, via the
//! kernel's `reboot` syscall (PSCI underneath) -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{fail, help, unknown_option};
use userlib::{ExitCode, LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART, reboot};

userlib::entry_with_args!(run);

const USAGE: &str = "poweroff [--reboot]";
const FLAGS: &[(&str, &str)] = &[("--reboot", "restart instead of powering off")];

fn run(args: userlib::Args) -> ExitCode {
    let mut cmd = LINUX_REBOOT_CMD_POWER_OFF;
    let mut what = "cannot power off";
    for arg in args.skip(1) {
        match arg {
            "--help" => return help(USAGE, FLAGS),
            "--reboot" => {
                cmd = LINUX_REBOOT_CMD_RESTART;
                what = "cannot restart";
            }
            _ => return unknown_option("poweroff", arg),
        }
    }
    // Only reachable if the kernel somehow rejected a command this program always passes correctly.
    let err = reboot(cmd);
    fail("poweroff", what, err);
    ExitCode(1)
}
