//! `poweroff [--reboot]` -- powers off the machine, or restarts it with `--reboot`, via the
//! kernel's `reboot` syscall (PSCI underneath) -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{fail, help};
use userlib::{ExitCode, LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART, reboot};
use getargs::Arg;
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "poweroff [--reboot]";
const FLAGS: &[(&str, &str)] = &[("--reboot", "restart instead of powering off")];

/// Reads the arguments: `--reboot` (restart instead) and `--help`; no operands. Returns the reboot
/// command and how to word a failure of it.
fn parse(args: userlib::Args) -> Result<(u32, &'static str), ExitCode> {
    let mut command = (LINUX_REBOOT_CMD_POWER_OFF, "cannot power off");
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("poweroff", &mut opts)? {
        match arg {
            Arg::Long("help") => return Err(help(USAGE, FLAGS)),
            Arg::Long("reboot") => command = (LINUX_REBOOT_CMD_RESTART, "cannot restart"),
            other => return Err(cli::invalid("poweroff", other)),
        }
    }
    Ok(command)
}

fn run(args: userlib::Args) -> ExitCode {
    let (cmd, what) = match parse(args) {
        Ok(command) => command,
        Err(status) => return status,
    };
    // Only reachable if the kernel somehow rejected a command this program always passes correctly.
    let err = reboot(cmd);
    fail("poweroff", what, err);
    ExitCode(1)
}
