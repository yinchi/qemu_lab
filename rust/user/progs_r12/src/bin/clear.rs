//! `clear` -- clears the screen and puts the cursor at the top left, with the console `ioctl`. Takes no
//! arguments. Fails with `Inappropriate ioctl for device` if stdout is not the console.

#![no_std]
#![no_main]

use userlib::{CONSOLE_CLEAR, ExitCode};

userlib::entry_with_args!(run);

const USAGE: &str = "clear";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: userlib::Args) -> ExitCode {
    if args.nth(1) == Some("--help") {
        return progs::help(USAGE, FLAGS);
    }
    let result = userlib::ioctl(1, CONSOLE_CLEAR, 0);
    if result < 0 {
        progs::fail("clear", "standard output", result);
        return ExitCode(1);
    }
    ExitCode(0)
}
