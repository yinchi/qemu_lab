//! `mkdir DIR...` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::diag;
use userlib::{ExitCode, mkdir};
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir DIR...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("mkdir", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if plain.count == 0 {
        return diag::missing_operand("mkdir");
    }

    let mut status = 0;
    for arg in cli::operands(args) {
        let result = mkdir(arg);
        if result < 0 {
            diag::cannot("mkdir", "create directory", arg, result);
            status = 1;
        }
    }
    ExitCode(status)
}
