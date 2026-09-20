//! `false` -- exits 1; see `docs/progs.md`.

#![no_std]
#![no_main]

userlib::entry!(run);

fn run() -> userlib::ExitCode {
    userlib::ExitCode(1)
}
