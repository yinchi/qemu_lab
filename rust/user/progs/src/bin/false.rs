//! `false` -- exits 1; see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::help;
use userlib::ExitCode;

userlib::entry_with_args!(run);

const USAGE: &str = "false";
const FLAGS: &[(&str, &str)] = &[];

fn run(args: userlib::Args) -> ExitCode {
    // Every other argument is ignored, matching real `false`.
    if args.skip(1).next() == Some("--help") {
        return help(USAGE, FLAGS);
    }
    ExitCode(1)
}
