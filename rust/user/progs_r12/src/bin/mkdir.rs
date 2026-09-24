//! `mkdir DIR...` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{diag, help};
use userlib::{ExitCode, mkdir};

userlib::entry_with_args!(run);

const USAGE: &str = "mkdir DIR...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let mut any = false;
    let mut status = 0;

    for arg in args.skip(1) {
        if arg == "--help" {
            return help(USAGE, FLAGS);
        }
        if arg.len() > 1 && arg.starts_with('-') {
            return diag::invalid_option("mkdir", arg);
        }
        any = true;
        let result = mkdir(arg);
        if result < 0 {
            diag::cannot("mkdir", "create directory", arg, result);
            status = 1;
        }
    }

    if !any {
        return diag::missing_operand("mkdir");
    }

    ExitCode(status)
}
