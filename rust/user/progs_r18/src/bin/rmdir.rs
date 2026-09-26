//! `rmdir DIR...` -- see `docs/progs.md`. Removes empty directories, the safe counterpart of `rm -r`: the kernel's
//! `unlinkat` with `AT_REMOVEDIR` refuses a directory that has anything in it (`Directory not empty`) and anything that
//! is not a directory (`Not a directory`). Every operand is tried; the status is 1 if any failed.

#![no_std]
#![no_main]

use progs::diag;
use progs_r12::cli;
use userlib::{ExitCode, unlink};

userlib::entry_with_args!(run);

const USAGE: &str = "rmdir DIR...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("rmdir", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        return diag::missing_operand("rmdir");
    }

    let mut status = 0;
    for dir in cli::operands(args) {
        let result = unlink(dir, true);
        if result < 0 {
            diag::report("rmdir", "failed to remove", dir, result);
            status = 1;
        }
    }
    ExitCode(status)
}
