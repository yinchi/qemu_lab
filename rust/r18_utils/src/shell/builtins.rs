//! The commands the shell runs itself instead of launching a program: the ones that have to change the
//! shell's own state. `cd`, `export`/`unset`, plus `source`/`.` and `sh` (Step 9's scripts -- see `shell::run_script_content`
//! for the interpreter, this module for how each finds its file). A builtin reports a problem as one
//! line of text, `cd: <what>`, which the caller prints the way `launch` prints its own errors.

use alloc::format;
use alloc::string::String;

use crate::exec::shell_state;
use crate::fs::blkio::VOL;
use crate::fs::{files, read_file_checked};
use crate::shell::{path_search, run_script_content};
use crate::static_ref;
use abi::errno::{EISDIR, ENOENT, ENOTDIR, errmsg};
use hadris_fat::sync::FileEntry;

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
/// `run_script_content`. `Ok` carries the exit status: `0`, except for a script, which is its last line's.
/// `Err` is what to report; the status is then `1`.
pub fn run(name: &str, args: &[&str], depth: usize) -> Result<i32, String> {
    match name {
        "cd" => cd(args).map(|()| 0),
        "export" => export(args).map(|()| 0),
        "unset" => unset(args).map(|()| 0),
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
            Ok(0)
        }
        _ => unreachable!("`is_builtin` said {name} is not one"),
    }
}

/// Finds the file `source` or `.` names, as bash does: a name **without a `/`** is looked up in the directories of
/// `$PATH` first (`path_search`, in order; the file need only be readable, **not executable**, and a directory of
/// that name is skipped) and, if none has it, in the working directory (bash's non-POSIX fallback). A name with a
/// `/` is used as written, against the working directory, and never searched. `None` if a bare name is in no
/// `PATH` directory, so the caller falls back to the working directory.
fn search_path_for(name: &str) -> Result<Option<FileEntry>, isize> {
    if name.contains('/') {
        return Ok(None);
    }
    let path = shell_state::frames().top().var("PATH").map(String::from);
    for candidate in path_search::candidates(path.as_deref(), name) {
        let Ok(absolute) = shell_state::absolute(&candidate) else { continue };
        match files::lookup(&absolute) {
            Ok(entry) if entry.is_directory() => {}
            Ok(entry) => return Ok(Some(entry)),
            Err(ENOENT | ENOTDIR | EISDIR) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Runs the script `path` names. `source` and `.` (`search_path`) find a bare name through `$PATH` before the working
/// directory (`search_path_for`); `sh FILE` -- like a script run by name -- takes the path as given, relative to the
/// working directory. No exec bit is needed either way (POSIX's rule for `source`/`.` and `sh FILE`).
/// `cmd` (`"source"`, `"."` or `"sh"`) only names the caller, for the error prefix.
fn run_script_file(cmd: &str, path: &str, scoped: bool, depth: usize) -> Result<i32, String> {
    let fail = |e: isize| format!("{cmd}: {path}: {}", errmsg(e));
    let found = if scoped { None } else { search_path_for(path).map_err(fail)? };
    let entry = match found {
        Some(entry) => entry,
        None => {
            let abspath = shell_state::absolute(path).map_err(fail)?;
            files::lookup(&abspath).map_err(fail)?
        }
    };
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
/// working directory; with no operand it goes to `$HOME` (`cd: HOME not set` if there is none, or it is empty).
/// `cd -` (needs `$OLDPWD`) and `-L`/`-P` (there are no symbolic links to choose about) are refused with a
/// clear error, as is more than one operand. On any error the working directory is unchanged. `args` excludes
/// the command word.
fn cd(args: &[&str]) -> Result<(), String> {
    let home;
    let target: &str = match args {
        [] => {
            home = shell_state::frames()
                .top()
                .var("HOME")
                .filter(|home| !home.is_empty())
                .map(String::from)
                .ok_or_else(|| String::from("cd: HOME not set"))?;
            &home
        }
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
