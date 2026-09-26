//! `umount TARGET` -- see `docs/progs.md`. Unmounts the volume mounted at TARGET (`umount` syscall: the root, a mount with
//! another inside it, and a volume with open files or a working directory in it are refused as busy).

#![no_std]
#![no_main]

use core::fmt::Write;

use abi::errno::EINVAL;
use progs::{Fd, diag, errmsg};
use progs_r12::cli;
use userlib::{ExitCode, umount};

userlib::entry_with_args!(run);

const USAGE: &str = "umount TARGET";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("umount", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    match plain.count {
        0 => diag::missing_operand("umount"),
        1 => {
            let target = plain.first.unwrap_or("");
            let result = umount(target);
            if result >= 0 {
                return ExitCode(0);
            }
            let why = if result == EINVAL { "not a mount point" } else { errmsg(result) };
            let _ = writeln!(Fd(2), "umount: {target}: {why}");
            ExitCode(1)
        }
        _ => diag::extra_operand("umount", plain.last.unwrap_or("")),
    }
}
