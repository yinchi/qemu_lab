//! The commands the shell runs itself instead of launching a program: the ones that have to change the
//! shell's own state. `cd`, plus `source`/`.` and `sh` (Step 9's scripts -- see `shell::run_script_content`
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
    matches!(name, "cd" | "source" | "." | "sh")
}

/// Runs the builtin `name` (one `is_builtin` says yes to) with `args`, the words after it. `depth` is
/// `run_line`'s script-nesting count, threaded through for `source`/`sh` to pass to
/// `run_script_content`.
pub fn run(name: &str, args: &[&str], depth: usize) -> Result<(), String> {
    match name {
        "cd" => cd(args),
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

/// `cd [DIR]`, POSIX's subset: `cd DIR` makes DIR (absolute, or relative to the working directory) the
/// working directory; with no operand it goes to `/` -- POSIX says `$HOME`, but there is no environment
/// until Stage 16, which then switches this to `$HOME`. `cd -` (needs `$OLDPWD`) and `-L`/`-P` (there
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
