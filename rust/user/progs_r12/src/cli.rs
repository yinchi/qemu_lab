//! Command-line parsing for the Stage 12 programs, on the `getargs` crate (no_std, no allocation).
//!
//! `getargs` produces a stream of short options (`-F`, and grouped, `-Fl`), long options (`--help`),
//! option values (`-n 5`, `-n5`, `--lines=5`) and operands, honors `--` and accepts a lone `-` as an
//! operand, with options allowed anywhere among the operands. It does not know which options exist:
//! each program says so with a `match`, and anything it does not name goes to `invalid`. This module
//! turns the parser's errors into GNU's messages (`progs::diag`) and adds the two shapes that repeat:
//!
//! - most programs take flags, then operands, and need the operands after *all* the flags are known
//!   (`rm a -r` recurses, as in GNU): parse once for the flags (`opts`/`next`), then walk the operands
//!   again (`operands`). That second walk skips options without looking at their values, so it is only
//!   for programs with no option that takes one -- `head` and `tail`, which do, parse once (`count_args`);
//! - `chmod` cannot use it for its mode, which looks like an option (`-x`), and parses by hand.

use core::fmt::Write;
use core::iter::Skip;

use getargs::{Arg, Error, Opt, Options};
use progs::{CountMode, Fd, atoi, diag, help};
use userlib::{Args, ExitCode};

/// A parser over a program's arguments after `argv[0]`.
pub type Opts = Options<&'static str, Skip<Args>>;

/// A fresh parser over `args` (which still holds `argv[0]`).
pub fn opts(args: Args) -> Opts {
    Options::new(args.skip(1))
}

/// A program's outcome: `Err` is a failure that has already been reported, so either way it is the status to exit with.
pub fn status(result: Result<ExitCode, ExitCode>) -> ExitCode {
    match result {
        Ok(status) | Err(status) => status,
    }
}

/// The next option or operand. A parse error (a missing or unwanted option value) has already been
/// reported, and comes back as the exit status to return.
pub fn next(prog: &str, opts: &mut Opts) -> Result<Option<Arg<&'static str>>, ExitCode> {
    opts.next_arg().map_err(|e| option_error(prog, e))
}

/// The value of the option just returned by `next`: `-n 5`, `-n5`, `--lines 5` and `--lines=5` all give `5`.
pub fn value(prog: &str, opts: &mut Opts) -> Result<&'static str, ExitCode> {
    opts.value().map_err(|e| option_error(prog, e))
}

/// Reports `arg` as not one of the program's options (or, for an operand, as one too many).
pub fn invalid(prog: &str, arg: Arg<&str>) -> ExitCode {
    match arg {
        Arg::Short(c) => diag::invalid_short(prog, c),
        Arg::Long(name) => diag::invalid_long(prog, name),
        Arg::Positional(operand) => diag::extra_operand(prog, operand),
    }
}

/// GNU's wording for the two ways an option's value can be wrong.
fn option_error(prog: &str, error: Error<&str>) -> ExitCode {
    let mut err = Fd(2);
    match error {
        Error::RequiresValue(Opt::Short(c)) => {
            let _ = writeln!(err, "{prog}: option requires an argument -- '{c}'");
        }
        Error::RequiresValue(Opt::Long(name)) => {
            let _ = writeln!(err, "{prog}: option '--{name}' requires an argument");
        }
        Error::DoesNotRequireValue(Opt::Long(name)) => {
            let _ = writeln!(err, "{prog}: option '--{name}' doesn't allow an argument");
        }
        Error::DoesNotRequireValue(Opt::Short(c)) => {
            let _ = writeln!(err, "{prog}: option '-{c}' doesn't allow an argument");
        }
        _ => {}
    }
    diag::try_help(prog);
    ExitCode(1)
}

/// The operands of a program none of whose options takes a value, in order; options are skipped
/// (they were checked by the first pass, `next`).
pub struct Operands(Opts);

/// An iterator over the operands in `args` (which still holds `argv[0]`).
pub fn operands(args: Args) -> Operands {
    Operands(opts(args))
}

impl Iterator for Operands {
    type Item = &'static str;

    fn next(&mut self) -> Option<&'static str> {
        loop {
            match self.0.next_arg() {
                Ok(Some(Arg::Positional(operand))) => return Some(operand),
                Ok(Some(_)) | Err(_) => continue,
                Ok(None) => return None,
            }
        }
    }
}

/// The operands of a program whose only option is `--help`: how many, the first two and the last.
pub struct Plain {
    pub count: usize,
    pub first: Option<&'static str>,
    pub second: Option<&'static str>,
    pub last: Option<&'static str>,
}

/// Validates the arguments of a program with no options besides `--help` and summarizes its operands.
/// `Err` is the status to return: `--help` (printed with `usage` and `flags`) or a problem already
/// reported. The operands themselves are then walked with `operands`.
pub fn plain(prog: &str, usage: &str, flags: &[(&str, &str)], args: Args) -> Result<Plain, ExitCode> {
    let mut opts = opts(args);
    let mut summary = Plain { count: 0, first: None, second: None, last: None };
    while let Some(arg) = next(prog, &mut opts)? {
        match arg {
            Arg::Long("help") => return Err(help(usage, flags)),
            Arg::Positional(operand) => {
                summary.count += 1;
                match summary.count {
                    1 => summary.first = Some(operand),
                    2 => summary.second = Some(operand),
                    _ => {}
                }
                summary.last = Some(operand);
            }
            other => return Err(invalid(prog, other)),
        }
    }
    Ok(summary)
}

/// What `head` and `tail` share: `[-n N | -c N] [file]`.
pub struct CountArgs {
    pub mode: CountMode,
    pub count: usize,
    pub file: Option<&'static str>,
}

/// Parses `head`'s and `tail`'s arguments: `-n N`/`--lines=N` or `-c N`/`--bytes=N` (the two exclude
/// each other) and at most one file. `Err` is the status to return: `--help` (printed with `usage` and
/// `flags`) or a problem already reported.
pub fn count_args(
    prog: &str,
    usage: &str,
    flags: &[(&str, &str)],
    args: Args,
) -> Result<CountArgs, ExitCode> {
    let mut opts = opts(args);
    let mut mode = None;
    let mut count = 10;
    let mut file = None;
    while let Some(arg) = next(prog, &mut opts)? {
        let (which, noun) = match arg {
            Arg::Long("help") => return Err(help(usage, flags)),
            Arg::Short('n') | Arg::Long("lines") => (CountMode::Lines, "lines"),
            Arg::Short('c') | Arg::Long("bytes") => (CountMode::Bytes, "bytes"),
            Arg::Positional(operand) => {
                if file.replace(operand).is_some() {
                    return Err(diag::extra_operand(prog, operand));
                }
                continue;
            }
            other => return Err(invalid(prog, other)),
        };
        if mode.replace(which).is_some() {
            let mut err = Fd(2);
            let _ = writeln!(err, "{prog}: options '-n' and '-c' are mutually exclusive");
            diag::try_help(prog);
            return Err(ExitCode(1));
        }
        let text = value(prog, &mut opts)?;
        // A negative count (GNU: "all but the last N") is not supported, so it is an invalid number.
        let Some(n) = atoi(text) else {
            let _ = writeln!(Fd(2), "{prog}: invalid number of {noun}: '{text}'");
            return Err(ExitCode(1));
        };
        count = n;
    }
    Ok(CountArgs { mode: mode.unwrap_or(CountMode::Lines), count, file })
}
