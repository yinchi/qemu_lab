//! `echo [-n] args...`: prints its arguments back, space-separated, followed by a newline unless
//! `-n` suppresses it -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::help;

userlib::entry_with_args!(run);

const USAGE: &str = "echo [-n] args...";
const FLAGS: &[(&str, &str)] = &[("-n", "suppress the trailing newline")];

fn run(args: userlib::Args) -> userlib::ExitCode {
    // args[0] is echo's own name, not part of what it echoes.
    let mut args = args.skip(1);

    let mut newline = true;
    let mut first_arg = args.next();
    if first_arg == Some("--help") {
        return help(USAGE, FLAGS);
    }
    if first_arg == Some("-n") {
        newline = false;
        first_arg = args.next();
    }

    // If there are any arguments, print them.
    if let Some(first) = first_arg {
        // Print the first argument without a leading space.
        userlib::write(1, first.as_bytes());
        // Print the remaining arguments, each preceded by a space.
        for arg in args {
            userlib::write(1, b" ");
            userlib::write(1, arg.as_bytes());
        }
    }

    if newline {
        userlib::write(1, b"\n");
    }
    userlib::ExitCode(0)
}
