//! `ls [-F] [dir]` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use core::fmt::Write;

use progs::{Fd, fail, unknown_option, usage};
use userlib::{ATTR_DIRECTORY, ATTR_EXEC, DIRENT_SIZE, DirEnt, ExitCode, O_RDONLY, close, getdents, open};

userlib::entry_with_args!(run);

/// How many records to fetch per `getdents` call.
const BATCH: usize = 8;

fn run(args: userlib::Args) -> ExitCode {
    let mut classify = false;
    let mut dir = None;

    // Parse command-line arguments to determine the options and the directory.
    for arg in args.skip(1) {

        // Check for -F: enable classification of file types.
        if arg == "-F" {
            classify = true;
        // All other options are unknown.
        } else if arg.len() > 1 && arg.starts_with('-') {
            return unknown_option("ls", arg);
        // If a non-option argument is encountered, it is treated as the directory.
        // Only one non-option argument (the directory) is allowed, more than one will
        // result in a usage error.
        } else if dir.replace(arg).is_some() {
            return usage("ls [-F] [dir]");
        }
    }
    // Use the specified directory or default to the root directory if none was provided.
    let dir = dir.unwrap_or("/");

    // Open the directory for reading.
    let fd = open(dir, O_RDONLY);
    if fd < 0 {
        fail("ls", dir, fd);
        return ExitCode(1);
    }

    // Initialize the buffer for reading directory entries and the status code.
    // The buffer can process up to `BATCH` directory entries at a time.
    let mut buf = [0u8; DIRENT_SIZE * BATCH];
    let mut status = 0;

    loop {
        let n = getdents(fd as usize, &mut buf);

        // Negative n: indicates an error occurred while reading directory entries.
        if n < 0 {
            fail("ls", dir, n);
            status = 1;
            break;
        }
        // Zero n: indicates end of directory entries.
        if n == 0 {
            break;
        }

        // Iterate over each directory entry in the buffer.
        for raw in buf[..n as usize].chunks_exact(DIRENT_SIZE) {
            let Some(ent) = DirEnt::parse(raw) else { continue };

            // If -F is specified, determine the appropriate suffix for the file type.
            let suffix = match (classify, ent.attrs) {
                (true, a) if a & ATTR_DIRECTORY != 0 => "/",
                (true, a) if a & ATTR_EXEC != 0 => "*",
                _ => "",
            };

            // Print the directory entry name followed by the appropriate suffix.
            let _ = writeln!(Fd(1), "{}{suffix}", ent.name);
        }
    }
    close(fd as usize);
    ExitCode(status)
}
