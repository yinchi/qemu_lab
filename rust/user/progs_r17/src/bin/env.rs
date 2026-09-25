//! `env` -- prints the environment, one `NAME=VALUE` line per variable. GNU's `env` also runs a command in a
//! modified environment (`env NAME=VALUE cmd`, `-i`, `-u`); the shell does that itself (`NAME=VALUE cmd`), so an
//! operand here is refused.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, diag};
use progs_r12::cli;
use userlib::{ExitCode, env};

userlib::entry_with_env!(run);

const USAGE: &str = "env";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("env", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if let Some(operand) = plain.first {
        return diag::extra_operand("env", operand);
    }
    let mut out = Fd(1);
    for (name, value) in env::vars() {
        let _ = writeln!(out, "{name}={value}");
    }
    ExitCode(0)
}
