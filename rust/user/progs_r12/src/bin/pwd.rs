//! `pwd` -- prints the working directory. POSIX's `-L` and `-P` (which of the logical and physical path
//! to print) are refused: there are no symbolic links to tell them apart.

#![no_std]
#![no_main]

use userlib::{ExitCode, PATH_MAX};
use getargs::Arg;
use progs_r12::cli;

userlib::entry_with_args!(run);

const USAGE: &str = "pwd";
const FLAGS: &[(&str, &str)] = &[];

/// Reads the arguments: `--help`, the two options this `pwd` refuses, and no operands.
fn parse(args: userlib::Args) -> Result<(), ExitCode> {
    use core::fmt::Write;
    let mut opts = cli::opts(args);
    // Every argument this `pwd` can be given ends it: `--help`, an option it refuses, or a stray operand.
    if let Some(arg) = cli::next("pwd", &mut opts)? {
        return Err(match arg {
            Arg::Long("help") => progs::help(USAGE, FLAGS),
            Arg::Short(flag @ ('L' | 'P')) => {
                let _ = writeln!(
                    progs::Fd(2),
                    "pwd: -{flag}: not supported (there are no symbolic links)"
                );
                ExitCode(1)
            }
            other => cli::invalid("pwd", other),
        });
    }
    Ok(())
}

fn run(args: userlib::Args) -> ExitCode {
    if let Err(status) = parse(args) {
        return status;
    }
    let mut buf = [0u8; PATH_MAX];
    let len = userlib::getcwd(&mut buf);
    if len < 0 {
        progs::fail("pwd", "cannot get the working directory", len);
        return ExitCode(1);
    }
    userlib::write(1, &buf[..len as usize]);
    userlib::write(1, b"\n");
    ExitCode(0)
}
