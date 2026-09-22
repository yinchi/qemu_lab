//! Starting a program from a command line: finding the file a command word names, checking that it
//! may run, running it, and reporting what went wrong (or its nonzero exit status) -- via
//! `shell_err` (`shell/mod.rs`), same as any other shell-reported error, so it is redirectable.

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

/// Runs the program `argv[0]` names -- see `find_program` -- with `argv` as its whole argument
/// list. Reports (in bash's wording) if:
///
/// - The program is not found (`command not found`).
/// - The program is a directory.
/// - The program is not marked as executable (`Permission denied`).
/// - The program is too large to be executed, or fails to load (`cannot execute: Exec format error`).
/// - The program exits with a nonzero status (`exit N`, standing in for `$?`).
pub fn launch(vol: &FatVolume<BlkIo>, argv: &[&str]) {
    let name = argv[0];
    let prog_entry = match find_program(name) {
        Ok(entry) => entry,
        Err(why) => {
            shell_err(&alloc::format!("{name}: {why}"));
            return;
        }
    };

    if prog_entry.is_directory() {
        shell_err(&alloc::format!("{name}: {}", errmsg(EISDIR)));
        return;
    }
    if prog_entry.attributes().bits() & ATTR_EXEC == 0 {
        shell_err(&alloc::format!("{name}: {}", errmsg(EACCES)));
        return;
    }
    if prog_entry.len() as usize > MAX_PROGRAM_SIZE {
        shell_err(&alloc::format!(
            "{name}: cannot execute: {}",
            errmsg(ENOEXEC)
        ));
        return;
    }

    let elf_bytes = match read_file_checked(vol, &prog_entry) {
        Ok(bytes) => bytes,
        Err(e) => {
            shell_err(&alloc::format!("{name}: {}", errmsg(e)));
            return;
        }
    };
    match process::run_program(&elf_bytes, argv) {
        Ok(0) => {}
        Ok(code) => shell_err(&alloc::format!("exit {code}")),
        Err(e) if e.errno() == E2BIG => shell_err(&alloc::format!("{name}: {}", errmsg(E2BIG))),
        Err(e) => shell_err(&alloc::format!(
            "{name}: cannot execute: {}",
            errmsg(e.errno())
        )),
    }
}
