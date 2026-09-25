//! `printenv [NAME]...` -- with no operand, prints the whole environment like `env`; with some, prints each named
//! variable's value on a line of its own. A name that is not set prints nothing, and makes the exit status 1 (GNU's).

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::Fd;
use progs_r12::cli;
use userlib::{ExitCode, env};

userlib::entry_with_env!(run);

const USAGE: &str = "printenv [NAME]...";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("printenv", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    let mut out = Fd(1);
    if plain.count == 0 {
        for (name, value) in env::vars() {
            let _ = writeln!(out, "{name}={value}");
        }
        return ExitCode(0);
    }
    let mut status = 0;
    for name in cli::operands(args) {
        match env::var(name) {
            Some(value) => {
                let _ = writeln!(out, "{value}");
            }
            None => status = 1,
        }
    }
    ExitCode(status)
}
