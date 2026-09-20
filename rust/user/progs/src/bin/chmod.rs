//! `chmod +x|-x|+w|-w file` -- see `docs/progs.md`.

#![no_std]
#![no_main]

use progs::{errmsg, usage};
use userlib::{ATTR_EXEC, ATTR_READ_ONLY, ExitCode, chmod};

userlib::entry_with_args!(run);

fn run(args: userlib::Args) -> ExitCode {
    let mut args = args.skip(1);

    // Expect: `chmod <mode> <file>`
    // Only one mode and one file argument are expected.
    let (Some(mode), Some(path), None) = (args.next(), args.next(), args.next()) else {
        return usage("chmod +x|-x|+w|-w file");
    };

    // Determine the set/clear bits to set for the specified mode.
    // FAT has no write bit, only a read-only bit, so `+w` clears it and `-w` sets it.
    let (set, clear) = match mode {
        "+x" => (ATTR_EXEC, 0),
        "-x" => (0, ATTR_EXEC),
        "+w" => (0, ATTR_READ_ONLY),
        "-w" => (ATTR_READ_ONLY, 0),
        _ => {
            use core::fmt::Write;
            let _ = writeln!(progs::Fd(2), "chmod: invalid mode: {mode}");
            return ExitCode(1);
        }
    };

    // Apply the set/clear bits to the specified file, or report an error if it fails.
    // Uses the helper function of the same name in `userlib`.
    let result = chmod(path, set, clear);
    if result < 0 {
        use core::fmt::Write;
        let _ = writeln!(progs::Fd(2), "chmod: {path}: {}", errmsg(result));
        return ExitCode(1);
    }
    ExitCode(0)
}
