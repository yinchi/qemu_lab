//! `echo`: prints its arguments back, space-separated, followed by a newline -- the minimal
//! program that actually consumes `argc`/`argv`, proving Stage 10's launcher end to end (see
//! `ROADMAP.md`'s Stage 10 section).

#![no_std]
#![no_main]

userlib::entry_with_args!(run);

fn run(args: userlib::Args) {
    // args[0] is echo's own name, not part of what it echoes.
    let mut args = args.skip(1);

    if let Some(first) = args.next() {
        userlib::write(1, first.as_bytes());
        for arg in args {
            userlib::write(1, b" ");
            userlib::write(1, arg.as_bytes());
        }
    }
    userlib::write(1, b"\n");
}
