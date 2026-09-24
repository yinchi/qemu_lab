//! `true` -- exits 0; see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::help;
use userlib::ExitCode;

userlib::entry_with_args!(run);

const USAGE: &str = "true";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: userlib::Args) -> ExitCode {
    // Every other argument is ignored, matching real `true`.
    if args.nth(1) == Some("--help") {
        return help(USAGE, FLAGS);
    }
    ExitCode(0)
}
