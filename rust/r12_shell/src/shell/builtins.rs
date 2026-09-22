//! The commands the shell runs itself instead of launching a program: the ones that have to change the
//! shell's own state. Only `cd` so far (`source` and `sh` come with scripts, Step 9). A builtin reports
//! a problem as one line of text, `cd: <what>`, which the caller prints the way `launch` prints its own
//! errors.

use alloc::format;
use alloc::string::String;

use crate::exec::shell_state;
use abi::errno::errmsg;

/// The builtin `name` is, if it is one.
pub fn is_builtin(name: &str) -> bool {
    name == "cd"
}

/// Runs the builtin `name` (one `is_builtin` says yes to) with `args`, the words after it.
pub fn run(name: &str, args: &[&str]) -> Result<(), String> {
    match name {
        "cd" => cd(args),
        _ => unreachable!("`is_builtin` said {name} is not one"),
    }
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
