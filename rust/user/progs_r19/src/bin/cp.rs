//! `cp [-p] [-r] [-n] [-v] SRC... DST` -- see `docs/progs.md`. Stage 19's tier adds `-p` (keep the modify time and the
//! executable and read-only bits) to Stage 18's `cp`, on the shared copy engine (`progs_r19::copy`), which also checks that
//! each copy came out the size of its source.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;

use abi::errno::ENOENT;
use alloc::string::String;
use getargs::Arg;
use progs::{Fd, basename, diag, help};
use progs_r12::cli;
use progs_r18::join;
use progs_r19::copy::Copier;
use userlib::{ATTR_DIRECTORY, ExitCode, stat};

userlib::entry_with_args!(run);

const USAGE: &str = "cp [-p] [-r] [-n] [-v] SRC... DST";
const FLAGS: &[(&str, &str)] = &[
    ("-p", "keep the modify time and the executable and read-only bits"),
    ("-r", "copy directories recursively"),
    ("-n", "do not overwrite an existing file"),
    ("-v", "print what is being copied"),
];

fn run(args: userlib::Args) -> ExitCode {
    cli::status(copy(args))
}

fn copy(args: userlib::Args) -> Result<ExitCode, ExitCode> {
    let mut o = Copier { prog: "cp", recursive: false, no_clobber: false, verbose: false, preserve: false };
    let (mut count, mut first, mut last) = (0usize, None, None);
    let mut opts = cli::opts(args);
    while let Some(arg) = cli::next("cp", &mut opts)? {
        match arg {
            Arg::Long("help") => return Ok(help(USAGE, FLAGS)),
            Arg::Short('p') | Arg::Long("preserve") => o.preserve = true,
            Arg::Short('r') | Arg::Short('R') | Arg::Long("recursive") => o.recursive = true,
            Arg::Short('n') | Arg::Long("no-clobber") => o.no_clobber = true,
            Arg::Short('v') | Arg::Long("verbose") => o.verbose = true,
            Arg::Positional(operand) => {
                count += 1;
                first.get_or_insert(operand);
                last = Some(operand);
            }
            other => return Err(cli::invalid("cp", other)),
        }
    }
    let Some(dst) = last.filter(|_| count >= 2) else {
        return Err(match first {
            Some(src) => diag::missing_destination_operand("cp", src),
            None => diag::missing_file_operand("cp"),
        });
    };

    let dst_is_dir = match stat(dst) {
        Ok(info) => info.attrs & ATTR_DIRECTORY != 0,
        Err(ENOENT) => false,
        Err(e) => {
            diag::cannot("cp", "stat", dst, e);
            return Ok(ExitCode(1));
        }
    };

    let n_src = count - 1;
    if n_src > 1 && !dst_is_dir {
        return Err(diag::target_not_directory("cp", dst));
    }

    let mut status = 0;
    for (i, src) in cli::operands(args).enumerate() {
        if i == count - 1 {
            break; // this operand is dst itself
        }

        let joined: String;
        let target: &str = if dst_is_dir {
            joined = join(dst, basename(src));
            &joined
        } else {
            dst
        };

        if src == target {
            let _ = writeln!(Fd(2), "cp: '{src}' and '{target}' are the same file");
            status = 1;
            continue;
        }

        if o.operand(src, target).is_err() {
            status = 1;
        }
    }

    Ok(ExitCode(status))
}
