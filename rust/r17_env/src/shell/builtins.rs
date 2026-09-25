//! The commands the shell runs itself instead of launching a program: the ones that have to change the
//! shell's own state. `cd`, `export`/`unset`, plus `source`/`.` and `sh` (Step 9's scripts -- see `shell::run_script_content`
//! for the interpreter, this module for how each finds its file). A builtin reports a problem as one
//! line of text, `cd: <what>`, which the caller prints the way `launch` prints its own errors.

use alloc::format;
use alloc::string::String;

use crate::exec::shell_state;
use crate::fs::blkio::VOL;
use crate::fs::{files, read_file_checked};
use crate::shell::run_script_content;
use crate::static_ref;
use abi::errno::{EISDIR, errmsg};

/// The builtin `name` is, if it is one.
pub fn is_builtin(name: &str) -> bool {
    #[cfg(feature = "testhooks")]
    if name == OVERFLOW_KERNEL_STACK {
        return true;
    }
    matches!(name, "cd" | "source" | "." | "sh" | "export" | "unset")
}

/// Test-only builtin (cargo feature `testhooks`): recurses until the kernel stack overflows into its
/// guard, so `test/cases/stack_guard.py` can check the overflow is reported instead of corrupting
/// memory. Nothing outside a test build has a command by this name.
#[cfg(feature = "testhooks")]
const OVERFLOW_KERNEL_STACK: &str = "__overflow_kernel_stack";

/// One kernel stack frame of `overflow`: big enough to cross the 1 MiB stack in a few hundred calls,
/// small enough (well under the 64 KiB guard) that no frame can step over the guard without touching it.
#[cfg(feature = "testhooks")]
#[allow(unconditional_recursion)]
#[inline(never)]
fn overflow(depth: usize) -> usize {
    let frame = [depth as u8; 2048];
    core::hint::black_box(&frame);
    overflow(depth + 1) + frame[depth % 2048] as usize
}

/// Runs the builtin `name` (one `is_builtin` says yes to) with `args`, the words after it. `depth` is
/// `run_line`'s script-nesting count, threaded through for `source`/`sh` to pass to
/// `run_script_content`.
pub fn run(name: &str, args: &[&str], depth: usize) -> Result<(), String> {
    match name {
        "cd" => cd(args),
        "export" => export(args),
        "unset" => unset(args),
        "source" | "." => match args {
            [path] => run_script_file(name, path, false, depth),
            [] => Err(format!("{name}: usage: {name} FILE")),
            _ => Err(format!("{name}: too many arguments")),
        },
        "sh" => match args {
            [path] => run_script_file(name, path, true, depth),
            [] => Err(String::from("sh: usage: sh FILE")),
            _ => Err(String::from("sh: too many arguments")),
        },
        #[cfg(feature = "testhooks")]
        OVERFLOW_KERNEL_STACK => {
            core::hint::black_box(overflow(0));
            Ok(())
        }
        _ => unreachable!("`is_builtin` said {name} is not one"),
    }
}

/// Resolves `path` against the working directory (unlike launching a program: no `/bin` search, and
/// no exec bit required -- POSIX's rule for both `source`/`.` and `sh FILE`) and runs it as a script.
/// `cmd` (`"source"`, `"."` or `"sh"`) only names the caller, for the error prefix.
fn run_script_file(cmd: &str, path: &str, scoped: bool, depth: usize) -> Result<(), String> {
    let abspath =
        shell_state::absolute(path).map_err(|e| format!("{cmd}: {path}: {}", errmsg(e)))?;
    let entry = files::lookup(&abspath).map_err(|e| format!("{cmd}: {path}: {}", errmsg(e)))?;
    if entry.is_directory() {
        return Err(format!("{cmd}: {path}: {}", errmsg(EISDIR)));
    }
    // SAFETY: VOL is populated before any program can run (kernel_main) and never cleared; builtins
    // run from the same single-threaded, non-reentrant shell loop as everything else that reads it.
    let vol = unsafe { static_ref!(VOL) };
    let bytes =
        read_file_checked(vol, &entry).map_err(|e| format!("{cmd}: {path}: {}", errmsg(e)))?;
    let content =
        core::str::from_utf8(&bytes).map_err(|_| format!("{cmd}: {path}: not valid UTF-8"))?;
    run_script_content(content, scoped, depth).map_err(|e| format!("{cmd}: {path}: {e}"))
}

/// `export NAME[=VALUE]...`: marks each variable exported -- handed to every program the shell starts and to
/// scripts run as their own process -- and, with a `=VALUE`, assigns it first. `export NAME` for a variable that
/// is not set does nothing (there is nothing to mark). Every operand is attempted; each invalid name is reported
/// (bash's wording) and makes the command fail. `export` alone (bash lists the exports) and `-p` are refused.
fn export(args: &[&str]) -> Result<(), String> {
    if args.is_empty() {
        return Err(String::from("export: usage: export NAME[=VALUE]..."));
    }
    let mut problems = String::new();
    for arg in args {
        if arg.len() > 1 && arg.starts_with('-') {
            return Err(format!("export: {arg}: invalid option"));
        }
        let (name, value) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (*arg, None),
        };
        if shell_state::frames().top_mut().export_var(name, value).is_err() {
            if !problems.is_empty() {
                problems.push('\n');
            }
            problems.push_str(&format!("export: '{arg}': not a valid identifier"));
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// `unset NAME...`: removes each variable, set or not (removing an unset one is not an error, as in POSIX).
/// A name that is not a valid identifier is reported. Options (`-v`, `-f`) are refused.
fn unset(args: &[&str]) -> Result<(), String> {
    let mut problems = String::new();
    for arg in args {
        if arg.len() > 1 && arg.starts_with('-') {
            return Err(format!("unset: {arg}: invalid option"));
        }
        if !crate::exec::frame_stack::is_valid_name(arg) {
            if !problems.is_empty() {
                problems.push('\n');
            }
            problems.push_str(&format!("unset: '{arg}': not a valid identifier"));
            continue;
        }
        shell_state::frames().top_mut().unset_var(arg);
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// `cd [DIR]`, POSIX's subset: `cd DIR` makes DIR (absolute, or relative to the working directory) the
/// working directory; with no operand it goes to `/` -- POSIX says `$HOME`, but there is no environment
/// until Stage 17, which then switches this to `$HOME`. `cd -` (needs `$OLDPWD`) and `-L`/`-P` (there
/// are no symbolic links to choose about) are refused with a clear error, as is more than one operand.
/// On any error the working directory is unchanged. `args` excludes the command word.
fn cd(args: &[&str]) -> Result<(), String> {
    let target = match args {
        [] => "/",
        ["-"] => return Err(String::from("cd: -: not supported (there is no $OLDPWD)")),
        ["-L"] | ["-P"] => {
            return Err(format!(
                "cd: {}: not supported (there are no symbolic links)",
                args[0]
            ));
        }
        [option, ..] if option.len() > 1 && option.starts_with('-') && *option != "--" => {
            return Err(format!("cd: {option}: invalid option"));
        }
        ["--", dir] | [dir] => dir,
        _ => return Err(String::from("cd: too many arguments")),
    };
    shell_state::chdir(target).map_err(|e| format!("cd: {target}: {}", errmsg(e)))
}
