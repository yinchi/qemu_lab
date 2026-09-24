//! `clear` -- clears the screen and puts the cursor at the top left, with the console `ioctl`. Takes no
//! arguments. Fails with `Inappropriate ioctl for device` if stdout is not the console.

#![no_std]
#![no_main]

use userlib::{CONSOLE_CLEAR, ExitCode};
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "clear";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("clear", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    if let Some(operand) = plain.first {
        return progs::diag::extra_operand("clear", operand);
    }
    let result = userlib::ioctl(1, CONSOLE_CLEAR, 0);
    if result < 0 {
        progs::fail("clear", "standard output", result);
        return ExitCode(1);
    }
    ExitCode(0)
}
