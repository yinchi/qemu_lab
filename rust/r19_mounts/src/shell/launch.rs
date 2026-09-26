//! Starting a program from a command line: finding the file a command word names, checking that it
//! may run, running it, and reporting what went wrong -- via
//! `shell_err` (`shell/mod.rs`), same as any other shell-reported error, so it is redirectable.

use alloc::string::String;
use alloc::vec::Vec;

use abi::errno::{E2BIG, EACCES, EISDIR, ENOENT, ENOEXEC, ENOTDIR, errmsg};
use abi::fs::ATTR_EXEC;

use crate::HEAP_SIZE;
use crate::exec::{process, shell_state};
use crate::fs::files::{self, Located};
use crate::shell::{path_search, shell_err};

/// The largest executable `launch` will read into memory: anything bigger is refused as not
/// executable rather than risking an allocation failure (which would panic the kernel).
const MAX_PROGRAM_SIZE: usize = HEAP_SIZE / 2;

/// Finds the file a command word names. A word containing `/` is a path, relative to the working
/// directory unless it starts with `/`. A bare name is looked up in the directories of `$PATH`
/// (`path_search.rs`: `/bin` if it is unset), in order, as typed: `cat` is `bin/cat`. (Stages 11-16 installed
/// programs as `cat.exe` and tried `name.exe` after the bare name; the name is exact now.) The
/// first regular file found wins; a directory of that name is skipped, as is a candidate that does not
/// exist. `Err` carries the text to report after `name: `.
fn find_program(name: &str) -> Result<Located, &'static str> {
    if name.contains('/') {
        return shell_state::absolute(name)
            .and_then(|path| files::lookup(&path))
            .map_err(errmsg);
    }
    let path = shell_state::frames().top().var("PATH").map(String::from);
    for candidate in path_search::candidates(path.as_deref(), name) {
        // A relative `PATH` entry is relative to the working directory; a candidate that cannot even be
        // named (too long) is one that does not exist.
        let Ok(absolute) = shell_state::absolute(&candidate) else { continue };
        match files::lookup(&absolute) {
            Ok(entry) if entry.is_directory() => {}
            Ok(entry) => return Ok(entry),
            Err(ENOENT | ENOTDIR | EISDIR) => {}
            Err(e) => return Err(errmsg(e)),
        }
    }
    Err("command not found")
}

/// The four bytes every ELF file starts with.
const ELF_MAGIC: &[u8] = b"\x7fELF";

/// How many leading bytes decide whether a non-ELF, exec-bit file is "binary" or "looks like a
/// script" -- bash's own `ENOEXEC` fallback heuristic (a NUL anywhere in that prefix means binary).
const SCRIPT_PROBE_LEN: usize = 128;

/// Exit statuses for a command that did not run, as in POSIX shells: `126` found but could not be run (a
/// directory, no exec bit, too big, not a valid program), `127` not found.
const STATUS_CANNOT_EXECUTE: i32 = 126;
const STATUS_NOT_FOUND: i32 = 127;

/// Runs the program `argv[0]` names -- see `find_program` -- with `argv` as its whole argument
/// list and the shell's exported variables as its environment. `depth` is `run_line`'s script-nesting count, passed to `run_script_content` if `argv[0]`
/// turns out to be a script rather than a program (see below). Reports (in bash's wording) if:
///
/// - The program is not found (`command not found`).
/// - The program is a directory.
/// - The program is not marked as executable (`Permission denied`).
/// - The program is too large to be executed, or fails to load (`cannot execute: Exec format error`).
/// - The program exits with a nonzero status.
///
/// An exec-bit file with no ELF magic is bash's `ENOEXEC` fallback, not an error: if its first
/// `SCRIPT_PROBE_LEN` bytes contain no NUL it "looks like" a script and is run as one, scoped like a
/// real child shell process (`./script` -- unlike `sh script`/`source script`, in `builtins.rs`, this
/// path already has the exec bit checked, so nothing further is needed there); genuine binary garbage
/// still reports `cannot execute binary file: Exec format error`. A script has no positional
/// parameters, so extra arguments are rejected the same way `sh` rejects
/// them, not silently ignored.
///
/// Returns the exit status: the program's, or a script's last line's, or `127` / `126` for a command that
/// could not run (every one of these already reported via `shell_err`). The status is not printed: it is
/// what `$?` shows.
pub fn launch(argv: &[&str], depth: usize) -> i32 {
    let name = argv[0];
    let prog_entry = match find_program(name) {
        Ok(entry) => entry,
        Err(why) => {
            shell_err(&alloc::format!("{name}: {why}"));
            // A name or path nothing answers to is "not found"; a lookup that failed for another reason (a path
            // through a file, one over the length limit) is not.
            let missing = why == "command not found" || why == errmsg(ENOENT);
            return if missing { STATUS_NOT_FOUND } else { STATUS_CANNOT_EXECUTE };
        }
    };

    if prog_entry.is_directory() {
        shell_err(&alloc::format!("{name}: {}", errmsg(EISDIR)));
        return STATUS_CANNOT_EXECUTE;
    }
    if prog_entry.attributes().bits() & ATTR_EXEC == 0 {
        shell_err(&alloc::format!("{name}: {}", errmsg(EACCES)));
        return STATUS_CANNOT_EXECUTE;
    }
    if prog_entry.len() as usize > MAX_PROGRAM_SIZE {
        shell_err(&alloc::format!(
            "{name}: cannot execute: {}",
            errmsg(ENOEXEC)
        ));
        return STATUS_CANNOT_EXECUTE;
    }

    let file_bytes = match prog_entry.read_all() {
        Ok(bytes) => bytes,
        Err(e) => {
            shell_err(&alloc::format!("{name}: {}", errmsg(e)));
            return STATUS_CANNOT_EXECUTE;
        }
    };

    if !file_bytes.starts_with(ELF_MAGIC) {
        return run_as_script_fallback(name, &file_bytes, argv, depth);
    }
    // The program's environment: what the shell has exported, as `NAME=VALUE` strings.
    let env: Vec<String> = shell_state::frames()
        .top()
        .exported()
        .map(|(name, value)| alloc::format!("{name}={value}"))
        .collect();
    let env: Vec<&str> = env.iter().map(String::as_str).collect();
    match process::run_program(&file_bytes, argv, &env) {
        Ok(status) => status,
        Err(e) if e.errno() == E2BIG => {
            shell_err(&alloc::format!("{name}: {}", errmsg(E2BIG)));
            STATUS_CANNOT_EXECUTE
        }
        Err(e) => {
            shell_err(&alloc::format!(
                "{name}: cannot execute: {}",
                errmsg(e.errno())
            ));
            STATUS_CANNOT_EXECUTE
        }
    }
}

/// `launch`'s `ENOEXEC` fallback: `file_bytes` has already been read and found to lack ELF magic. Returns the
/// status: the script's, or `126` if it could not be run as one.
fn run_as_script_fallback(name: &str, file_bytes: &[u8], argv: &[&str], depth: usize) -> i32 {
    let probe_len = file_bytes.len().min(SCRIPT_PROBE_LEN);
    if file_bytes[..probe_len].contains(&0) {
        shell_err(&alloc::format!(
            "{name}: cannot execute binary file: {}",
            errmsg(ENOEXEC)
        ));
        return STATUS_CANNOT_EXECUTE;
    }
    if argv.len() > 1 {
        shell_err(&alloc::format!("{name}: too many arguments"));
        return STATUS_CANNOT_EXECUTE;
    }
    let content = match core::str::from_utf8(file_bytes) {
        Ok(content) => content,
        Err(_) => {
            shell_err(&alloc::format!("{name}: not valid UTF-8"));
            return STATUS_CANNOT_EXECUTE;
        }
    };
    match crate::shell::run_script_content(content, true, depth) {
        Ok(status) => status,
        Err(e) => {
            shell_err(&alloc::format!("{name}: {e}"));
            STATUS_CANNOT_EXECUTE
        }
    }
}
