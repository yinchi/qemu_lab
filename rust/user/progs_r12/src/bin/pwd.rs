//! `pwd` -- prints the working directory. POSIX's `-L` and `-P` (which of the logical and physical path
//! to print) are refused: there are no symbolic links to tell them apart.

#![no_std]
#![no_main]

use userlib::{ExitCode, PATH_MAX};

userlib::entry_with_args!(run);

const USAGE: &str = "pwd";
const FLAGS: &[(&str, &str)] = &[];

fn run(mut args: userlib::Args) -> ExitCode {
    let _ = args.next(); // argv[0]
    if let Some(arg) = args.next() {
        return match arg {
            "--help" => progs::help(USAGE, FLAGS),
            "-L" | "-P" => {
                use core::fmt::Write;
                let _ = writeln!(
                    progs::Fd(2),
                    "pwd: {arg}: not supported (there are no symbolic links)"
                );
                ExitCode(1)
            }
            _ if arg.starts_with('-') => progs::diag::invalid_option("pwd", arg),
            _ => progs::diag::extra_operand("pwd", arg),
        };
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
