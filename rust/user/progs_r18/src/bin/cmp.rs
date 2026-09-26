//! `cmp [-s] FILE1 FILE2` -- see `docs/progs.md`. Compares two files byte by byte. Exit status 0 if they are the same, 1
//! if they differ (after naming the first byte that does, and its line), 2 if it could not compare them. `-` is standard
//! input. Meant for checking that a `cp` or `mv` kept a file whole, so it reads both files in one pass and holds neither.

#![no_std]
#![no_main]

use core::fmt::Write;

use getargs::Arg;
use progs::{CHUNK, Fd, Input, diag, fail, help};
use progs_r12::cli;
use userlib::{ExitCode, read};

userlib::entry_with_args!(run);

const USAGE: &str = "cmp [-s] FILE1 FILE2";
const FLAGS: &[(&str, &str)] = &[("-s", "print nothing; the exit status says whether they differ")];

/// Fills `buf` from `fd` as far as it goes (a `read` may return less, notably from stdin): the count, which is short only at
/// the end of the file. `Err` is the negative error.
fn fill(fd: usize, buf: &mut [u8]) -> Result<usize, isize> {
    let mut have = 0;
    while have < buf.len() {
        let n = read(fd, &mut buf[have..]);
        if n < 0 {
            return Err(n);
        }
        if n == 0 {
            break;
        }
        have += n as usize;
    }
    Ok(have)
}

fn run(args: userlib::Args) -> ExitCode {
    let mut silent = false;
    let mut names: [&str; 2] = [""; 2];
    let mut count = 0usize;
    let mut opts = cli::opts(args);
    loop {
        match cli::next("cmp", &mut opts) {
            Ok(None) => break,
            Ok(Some(Arg::Long("help"))) => return help(USAGE, FLAGS),
            Ok(Some(Arg::Short('s') | Arg::Long("quiet" | "silent"))) => silent = true,
            Ok(Some(Arg::Positional(name))) => {
                if count == 2 {
                    diag::extra_operand("cmp", name);
                    return ExitCode(2);
                }
                names[count] = name;
                count += 1;
            }
            Ok(Some(other)) => {
                cli::invalid("cmp", other);
                return ExitCode(2);
            }
            Err(_) => return ExitCode(2),
        }
    }
    if count < 2 {
        if count == 0 {
            diag::missing_operand("cmp");
        } else {
            diag::missing_operand_after("cmp", names[0]);
        }
        return ExitCode(2);
    }

    // `-` is standard input.
    let open = |name: &'static str| Input::open(if name == "-" { None } else { Some(name) });
    let (first, second) = match (open(names[0]), open(names[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) => {
            fail("cmp", names[0], e);
            return ExitCode(2);
        }
        (_, Err(e)) => {
            fail("cmp", names[1], e);
            return ExitCode(2);
        }
    };

    let mut a = [0u8; CHUNK];
    let mut b = [0u8; CHUNK];
    let mut offset = 0usize; // bytes already compared
    let mut line = 1usize; // the line the next byte is on, by the first file's newlines
    loop {
        let (na, nb) = match (fill(first.fd, &mut a), fill(second.fd, &mut b)) {
            (Ok(na), Ok(nb)) => (na, nb),
            (Err(e), _) => {
                fail("cmp", names[0], e);
                return ExitCode(2);
            }
            (_, Err(e)) => {
                fail("cmp", names[1], e);
                return ExitCode(2);
            }
        };
        let common = na.min(nb);
        for i in 0..common {
            if a[i] != b[i] {
                if !silent {
                    let _ = writeln!(Fd(1), "{} {} differ: byte {}, line {line}", names[0], names[1], offset + i + 1);
                }
                return ExitCode(1);
            }
            if a[i] == b'\n' {
                line += 1;
            }
        }
        offset += common;
        if na != nb {
            // One file ended first: it is a prefix of the other.
            if !silent {
                let shorter = if na < nb { names[0] } else { names[1] };
                let _ = writeln!(Fd(2), "cmp: EOF on {shorter} after byte {offset}");
            }
            return ExitCode(1);
        }
        if na < CHUNK {
            return ExitCode(0); // both ended together
        }
    }
}
