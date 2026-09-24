//! Diagnostics in the wording GNU coreutils uses, for the Stage 12 program tier (`progs_r12`, and the
//! base programs that tier replaces). The base tier's own programs keep `crate::fail`, `unknown_option`
//! and `usage`, whose text r09-r11's tests pin; nothing there calls this module.
//!
//! GNU's shapes, in the C locale (plain ASCII quotes):
//! - `prog: cannot VERB 'path': reason` for a failed operation (`cannot remove`, `cannot stat`, ...);
//! - `prog: invalid option -- 'x'` / `prog: unrecognized option '--foo'`, then a `Try` line;
//! - `prog: missing operand` / `extra operand 'x'`, then a `Try` line.
//!
//! Programs that GNU also words as a bare `prog: path: reason` (`cat`, `wc`, `tee`) keep using
//! `crate::fail`.

use core::fmt::Write;

use abi::errno::{EACCES, EMFILE, ENOENT, ENOTDIR};
use userlib::ExitCode;

use crate::{Fd, errmsg};

fn err() -> Fd {
    Fd(2)
}

/// `Try 'prog --help' for more information.`
pub fn try_help(prog: &str) {
    let _ = writeln!(err(), "Try '{prog} --help' for more information.");
}

/// `prog: invalid option -- 'x'` for a bad short option `c`, then the `Try` line.
pub fn invalid_short(prog: &str, c: char) -> ExitCode {
    let _ = writeln!(err(), "{prog}: invalid option -- '{c}'");
    try_help(prog);
    ExitCode(1)
}

/// A bad option argument `arg`: `--foo` is `unrecognized option`, and `-xyz` reports its first
/// letter (a program that reads a group of letters one by one names the bad one with `invalid_short`).
pub fn invalid_option(prog: &str, arg: &str) -> ExitCode {
    if arg.starts_with("--") {
        let _ = writeln!(err(), "{prog}: unrecognized option '{arg}'");
        try_help(prog);
        ExitCode(1)
    } else {
        invalid_short(prog, arg[1..].chars().next().unwrap_or('-'))
    }
}

/// `prog: missing operand`, then the `Try` line.
pub fn missing_operand(prog: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: missing operand");
    try_help(prog);
    ExitCode(1)
}

/// `prog: missing operand after 'arg'` -- an operand was given but a later one is needed.
pub fn missing_operand_after(prog: &str, arg: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: missing operand after '{arg}'");
    try_help(prog);
    ExitCode(1)
}

/// `cp`'s and `mv`'s wording when none of their file operands was given.
pub fn missing_file_operand(prog: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: missing file operand");
    try_help(prog);
    ExitCode(1)
}

/// `cp`'s and `mv`'s wording when only the source was given.
pub fn missing_destination_operand(prog: &str, after: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: missing destination file operand after '{after}'");
    try_help(prog);
    ExitCode(1)
}

/// `prog: extra operand 'arg'`, then the `Try` line.
pub fn extra_operand(prog: &str, arg: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: extra operand '{arg}'");
    try_help(prog);
    ExitCode(1)
}

/// `prog: target 'dst' is not a directory` -- several sources but a destination that is not a directory.
pub fn target_not_directory(prog: &str, dst: &str) -> ExitCode {
    let _ = writeln!(err(), "{prog}: target '{dst}' is not a directory");
    ExitCode(1)
}

/// `prog: what 'path': reason` -- `what` is the whole phrase, `cannot remove` or `error reading`.
pub fn report(prog: &str, what: &str, path: &str, code: isize) {
    let _ = writeln!(err(), "{prog}: {what} '{path}': {}", errmsg(code));
}

/// `prog: cannot VERB 'path': reason`.
pub fn cannot(prog: &str, verb: &str, path: &str, code: isize) {
    let _ = writeln!(err(), "{prog}: cannot {verb} '{path}': {}", errmsg(code));
}

/// A failure to get input from `path` (an open or a read, which the caller cannot always tell apart):
/// the ways an open fails are `cannot open 'path' for reading`, anything else `error reading 'path'`.
pub fn input_error(prog: &str, path: &str, code: isize) {
    if matches!(code, ENOENT | ENOTDIR | EACCES | EMFILE) {
        let _ = writeln!(err(), "{prog}: cannot open '{path}' for reading: {}", errmsg(code));
    } else {
        report(prog, "error reading", path, code);
    }
}

/// `prog: write error: reason`, for a failure writing standard output.
pub fn write_error(prog: &str, code: isize) {
    let _ = writeln!(err(), "{prog}: write error: {}", errmsg(code));
}
