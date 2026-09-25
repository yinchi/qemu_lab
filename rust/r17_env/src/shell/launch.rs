//! Starting a program from a command line: finding the file a command word names, checking that it
//! may run, running it, and reporting what went wrong (or its nonzero exit status) -- via
//! `shell_err` (`shell/mod.rs`), same as any other shell-reported error, so it is redirectable.

use alloc::string::String;
use alloc::vec::Vec;

use abi::errno::{E2BIG, EACCES, EISDIR, ENOENT, ENOEXEC, ENOTDIR, errmsg};
use abi::fs::ATTR_EXEC;
use hadris_fat::sync::{FatVolume, FileEntry};

use crate::HEAP_SIZE;
use crate::exec::{process, shell_state};
use crate::fs::blkio::BlkIo;
use crate::fs::{files, read_file_checked};
use crate::shell::shell_err;

/// The largest executable `launch` will read into memory: anything bigger is refused as not
/// executable rather than risking an allocation failure (which would panic the kernel).
const MAX_PROGRAM_SIZE: usize = HEAP_SIZE / 2;

/// Finds the file a command word names. A word containing `/` is a path, relative to the working
/// directory unless it starts with `/`. A bare name is looked up in `/bin` (nowhere else -- no
/// `PATH`-style search, and independent of the working directory), trying the bare name first and
/// then `name.exe`, Cygwin's own lookup order, so `cat` finds `bin/cat.exe` without the `.exe` ever
/// being typed. `Err` carries the text to report after `name: `.
fn find_program(name: &str) -> Result<FileEntry, &'static str> {
    if name.contains('/') {
        return shell_state::absolute(name)
            .and_then(|path| files::lookup(&path))
            .map_err(errmsg);
    }
    match files::lookup(&alloc::format!("/bin/{name}")) {
        Err(ENOENT | ENOTDIR) => {}
        found => return found.map_err(errmsg),
    }
    match files::lookup(&alloc::format!("/bin/{name}.exe")) {
        Err(ENOENT | ENOTDIR) => Err("command not found"),
        found => found.map_err(errmsg),
    }
}

/// The four bytes every ELF file starts with.
const ELF_MAGIC: &[u8] = b"\x7fELF";

/// How many leading bytes decide whether a non-ELF, exec-bit file is "binary" or "looks like a
/// script" -- bash's own `ENOEXEC` fallback heuristic (a NUL anywhere in that prefix means binary).
const SCRIPT_PROBE_LEN: usize = 128;

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
/// Returns `None` if no program actually ran (not found, not a file, not executable, too large,
/// failed to load, or ran as a script instead -- every one of these already reported via `shell_err`)
/// or `Some(code)` if one did. Printing `exit {code}` (our stand-in for `$?`, `Stage12.md`'s Step 11)
/// is the caller's decision, not this function's: a single command prints it unconditionally, a
/// pipeline only for its last stage.
pub fn launch(vol: &FatVolume<BlkIo>, argv: &[&str], depth: usize) -> Option<i32> {
    let name = argv[0];
    let prog_entry = match find_program(name) {
        Ok(entry) => entry,
        Err(why) => {
            shell_err(&alloc::format!("{name}: {why}"));
            return None;
        }
    };

    if prog_entry.is_directory() {
        shell_err(&alloc::format!("{name}: {}", errmsg(EISDIR)));
        return None;
    }
    if prog_entry.attributes().bits() & ATTR_EXEC == 0 {
        shell_err(&alloc::format!("{name}: {}", errmsg(EACCES)));
        return None;
    }
    if prog_entry.len() as usize > MAX_PROGRAM_SIZE {
        shell_err(&alloc::format!(
            "{name}: cannot execute: {}",
            errmsg(ENOEXEC)
        ));
        return None;
    }

    let file_bytes = match read_file_checked(vol, &prog_entry) {
        Ok(bytes) => bytes,
        Err(e) => {
            shell_err(&alloc::format!("{name}: {}", errmsg(e)));
            return None;
        }
    };

    if !file_bytes.starts_with(ELF_MAGIC) {
        run_as_script_fallback(name, &file_bytes, argv, depth);
        return None;
    }
    // The program's environment: what the shell has exported, as `NAME=VALUE` strings.
    let env: Vec<String> = shell_state::frames()
        .top()
        .exported()
        .map(|(name, value)| alloc::format!("{name}={value}"))
        .collect();
    let env: Vec<&str> = env.iter().map(String::as_str).collect();
    match process::run_program(&file_bytes, argv, &env) {
        Ok(code) => Some(code),
        Err(e) if e.errno() == E2BIG => {
            shell_err(&alloc::format!("{name}: {}", errmsg(E2BIG)));
            None
        }
        Err(e) => {
            shell_err(&alloc::format!(
                "{name}: cannot execute: {}",
                errmsg(e.errno())
            ));
            None
        }
    }
}

/// `launch`'s `ENOEXEC` fallback: `file_bytes` has already been read and found to lack ELF magic.
fn run_as_script_fallback(name: &str, file_bytes: &[u8], argv: &[&str], depth: usize) {
    let probe_len = file_bytes.len().min(SCRIPT_PROBE_LEN);
    if file_bytes[..probe_len].contains(&0) {
        shell_err(&alloc::format!(
            "{name}: cannot execute binary file: {}",
            errmsg(ENOEXEC)
        ));
        return;
    }
    if argv.len() > 1 {
        shell_err(&alloc::format!("{name}: too many arguments"));
        return;
    }
    let content = match core::str::from_utf8(file_bytes) {
        Ok(content) => content,
        Err(_) => {
            shell_err(&alloc::format!("{name}: not valid UTF-8"));
            return;
        }
    };
    if let Err(e) = crate::shell::run_script_content(content, true, depth) {
        shell_err(&alloc::format!("{name}: {e}"));
    }
}
