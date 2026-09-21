//! Starting a program from a command line: finding the file a command word names, checking that it
//! may run, running it, and reporting what went wrong (or its nonzero exit status) to the UART and
//! the console.

use abi::errno::{E2BIG, EISDIR, ENOENT, ENOEXEC, ENOTDIR, errmsg};
use abi::fs::ATTR_EXEC;
use hadris_fat::sync::{FatVolume, FileEntry};

use super::argv::Argv;
use crate::HEAP_SIZE;
use crate::console::{BG, Console, FG};
use crate::exec::process;
use crate::fs::blkio::BlkIo;
use crate::fs::{files, read_file_checked};
use crate::platform::uart::{uart_ensure_newline, uart_write};

/// The largest executable `launch` will read into memory: anything bigger is refused as not
/// executable rather than risking an allocation failure (which would panic the kernel).
const MAX_PROGRAM_SIZE: usize = HEAP_SIZE / 2;

/// Finds the file a command word names. A word containing `/` is a path, used as typed.
///
/// QUIRK, resolved by `Stage12.md`'s Step 6: there is no working directory yet, so every path is
/// resolved from the root of the disk -- `tests/probe.exe` and `/tests/probe.exe` are the same file,
/// and a leading `./` or any `.`/`..` component matches nothing. Once `cd` exists, a path without a
/// leading `/` resolves against the working directory instead. A bare name is looked up in
/// `bin/` (nowhere else -- no `PATH`-style search), trying the bare name first and then `name.exe`,
/// Cygwin's own lookup order, so `cat` finds `bin/cat.exe` without the `.exe` ever being typed.
/// `Err` carries the text to report after `name: `.
fn find_program(name: &str) -> Result<FileEntry, &'static str> {
    if name.contains('/') {
        return files::lookup(name).map_err(errmsg);
    }
    match files::lookup(&alloc::format!("bin/{name}")) {
        Err(ENOENT | ENOTDIR) => {}
        found => return found.map_err(errmsg),
    }
    match files::lookup(&alloc::format!("bin/{name}.exe")) {
        Err(ENOENT | ENOTDIR) => Err("not found"),
        found => found.map_err(errmsg),
    }
}

/// Runs the program `argv.program()` names -- see `find_program` -- with `argv`'s full argument
/// list. Reports an error if:
///
/// - The program is not found.
/// - The program is a directory.
/// - The program is not marked as executable.
/// - The program is too large to be executed.
/// - The program fails to load.
/// - The program exits with a nonzero status.
pub fn launch(vol: &FatVolume<BlkIo>, argv: &Argv, console: &mut Console) {
    let name = argv.program();
    let prog_entry = match find_program(name) {
        Ok(entry) => entry,
        Err(why) => {
            report(console, &alloc::format!("{name}: {why}"));
            return;
        }
    };

    if prog_entry.is_directory() {
        report(console, &alloc::format!("{name}: {}", errmsg(EISDIR)));
        return;
    }
    if prog_entry.attributes().bits() & ATTR_EXEC == 0 {
        report(console, &alloc::format!("{name}: not executable"));
        return;
    }
    if prog_entry.len() as usize > MAX_PROGRAM_SIZE {
        report(
            console,
            &alloc::format!("{name}: cannot execute: {}", errmsg(ENOEXEC)),
        );
        return;
    }

    let elf_bytes = match read_file_checked(vol, &prog_entry) {
        Ok(bytes) => bytes,
        Err(e) => {
            report(console, &alloc::format!("{name}: {}", errmsg(e)));
            return;
        }
    };
    match process::run_program(&elf_bytes, &argv.as_argv()) {
        Ok(0) => {}
        Ok(code) => report(console, &alloc::format!("exit {code}")),
        Err(e) if e.errno() == E2BIG => {
            report(console, &alloc::format!("{name}: {}", errmsg(E2BIG)))
        }
        Err(e) => report(
            console,
            &alloc::format!("{name}: cannot execute: {}", errmsg(e.errno())),
        ),
    }
}

/// Reports one line of text to both UART and the console -- used for the errors `launch` (and
/// `Argv::parse` failing) can report, so a typo'd or unbuilt command is visible on screen, not
/// only in the UART log a user may not even have open. Starts a new row/line first if a program's
/// last output left the cursor mid-line.
pub fn report(console: &mut Console, msg: &str) {
    uart_ensure_newline();
    uart_write(msg.as_bytes());
    uart_write(b"\n");
    if console.cursor().1 != 0 {
        console.write_char('\n', FG, BG);
    }
    for c in msg.chars() {
        console.write_char(c, FG, BG);
    }
    console.write_char('\n', FG, BG);
}
