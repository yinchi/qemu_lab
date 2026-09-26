//! `touch [-c] FILE...` -- see `docs/progs.md`. Creates a file that does not exist, and makes the modify time of one
//! that does the current time -- without changing what is in it.
//!
//! There is no syscall that sets a timestamp, so this does it the only way the kernel offers: opens the file for
//! appending and closes it again, writing nothing. Opening for write creates a missing file; an append open leaves an
//! existing file's data alone; and closing commits the file's directory entry, which is stamped with the clock's time
//! (`hadris-fat`'s `FileWriter::finish`). That is why there is no `-d`, `-t` or `-r`, and why a directory cannot be touched.

#![no_std]
#![no_main]

use getargs::Arg;
use progs::{diag, help};
use progs_r12::cli;
use userlib::{ExitCode, O_APPEND, O_WRONLY, close, open, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "touch [-c] FILE...";
const FLAGS: &[(&str, &str)] = &[("-c", "do not create a file that does not exist")];

fn run(args: userlib::Args) -> ExitCode {
    cli::status(touch(args))
}

fn touch(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut no_create = false;
    let mut files = 0usize;
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("touch", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('c') | Arg::Long("no-create") => no_create = true,
            Arg::Positional(_) => files += 1,
            other => return Err(cli::invalid("touch", other)),
        }
    }
    if files == 0 {
        return Err(diag::missing_file_operand("touch"));
    }

    let mut status = 0;
    for path in cli::operands(args) {
        // `-c`: leave a missing file missing, silently.
        if no_create {
            match stat(path) {
                Ok(_) => {}
                Err(abi::errno::ENOENT) => continue,
                Err(e) => {
                    diag::cannot("touch", "touch", path, e);
                    status = 1;
                    continue;
                }
            }
        }
        let fd = open(path, O_WRONLY | O_APPEND);
        if fd < 0 {
            diag::cannot("touch", "touch", path, fd);
            status = 1;
            continue;
        }
        close(fd as usize);
    }
    Ok(ExitCode(status))
}
