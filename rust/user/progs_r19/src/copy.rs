//! Copying files and directory trees -- what `cp` does, and what `mv` does between two volumes. The engine reports in GNU's
//! wording under the name of the program that uses it (`Copier::prog`), and its failures are `Err(Reported)` after the
//! report, so a caller only decides what to do next.
//!
//! A directory is copied by creating the destination directory and copying its entries into it, in name order; an
//! existing destination directory is merged into. Every file's size is checked once it is closed: a copy that came up short
//! (a full disk) is an error, not a quiet truncation. With `preserve`, a copied file keeps its executable and read-only bits
//! and every copy keeps its modify time -- a directory's after its contents are in, since putting entries in
//! may touch it -- through `utimens` (Stage 19).

extern crate alloc;

use core::fmt::Write;

use abi::errno::{EISDIR, ENOENT};
use progs::{CHUNK, Fd, diag, write_all};
use userlib::{
    ATTR_DIRECTORY, ATTR_EXEC, ATTR_READ_ONLY, O_RDONLY, O_WRONLY, Stat, TimeSet, chmod, close, mkdir, open, read, stat, unlink, utimens,
};

use crate::stamp::fat_to_unix;
use progs_r18::{join, read_dir};

/// A copy that failed, and has already said why: the caller only decides what to do next.
#[derive(Debug)]
pub struct Reported;

/// What a copy does, and who is talking.
pub struct Copier {
    /// The program's name, for its messages.
    pub prog: &'static str,
    pub recursive: bool,
    pub no_clobber: bool,
    pub verbose: bool,
    /// Keep the executable and read-only bits, and the modify time.
    pub preserve: bool,
}

impl Copier {
    fn announce(&self, src: &str, dst: &str) {
        if self.verbose {
            let _ = writeln!(Fd(1), "'{src}' -> '{dst}'");
        }
    }

    /// Gives `dst` what `preserve` asks of a copy of `src`: for a file, its executable and read-only bits; for both, its times.
    fn preserve_from(&self, info: &Stat, dst: &str, is_dir: bool) -> Result<(), Reported> {
        if !self.preserve {
            return Ok(());
        }
        let mut result = Ok(());
        if !is_dir {
            const BITS: u8 = ATTR_EXEC | ATTR_READ_ONLY;
            let want = info.attrs & BITS;
            let r = chmod(dst, want, BITS & !want);
            if r < 0 {
                diag::report(self.prog, "failed to preserve permissions for", dst, r);
                result = Err(Reported);
            }
        }
        let mtime = fat_to_unix(info.modified_date, info.modified_time).map_or(TimeSet::Omit, TimeSet::At);
        let r = utimens(dst, TimeSet::Omit, mtime);
        if r < 0 {
            diag::report(self.prog, "failed to preserve timestamps for", dst, r);
            result = Err(Reported);
        }
        result
    }

    /// Copies the file `src` to `dst`, reporting in GNU's wording (attributed to whichever side actually failed) and
    /// returning `Err(Reported)` if it didn't work. Reads the first chunk *before* creating (and so truncating) `dst`: a source that
    /// can't be read at all must not destroy an existing destination first.
    pub fn file(&self, src: &str, dst: &str) -> Result<(), Reported> {
        let prog = self.prog;
        if self.no_clobber && stat(dst).is_ok() {
            return Ok(());
        }
        let input = open(src, O_RDONLY);
        if input < 0 {
            if input == ENOENT {
                diag::cannot(prog, "stat", src, input);
            } else {
                diag::input_error(prog, src, input);
            }
            return Err(Reported);
        }
        let input = input as usize;

        let mut buf = [0u8; CHUNK];
        let first = read(input, &mut buf);
        if first < 0 {
            if first == EISDIR {
                let _ = writeln!(Fd(2), "{prog}: -r not specified; omitting directory '{src}'");
            } else {
                diag::report(prog, "error reading", src, first);
            }
            close(input);
            return Err(Reported);
        }

        let output = open(dst, O_WRONLY);
        if output < 0 {
            diag::cannot(prog, "create regular file", dst, output);
            close(input);
            return Err(Reported);
        }
        let output = output as usize;

        let mut status = Ok(());
        let mut total: u64 = 0;
        let mut n = first;
        while n > 0 {
            if let Err(e) = write_all(output, &buf[..n as usize]) {
                diag::report(prog, "error writing", dst, e);
                status = Err(Reported);
                break;
            }
            total += n as u64;
            n = read(input, &mut buf);
            if n < 0 {
                diag::report(prog, "error reading", src, n);
                status = Err(Reported);
                break;
            }
        }
        close(input);

        // Closing is what commits the file's size to disk -- a failure here is a failed copy.
        let closed = close(output);
        if closed < 0 && status.is_ok() {
            diag::report(prog, "error writing", dst, closed);
            status = Err(Reported);
        }
        // And what is on the disk must be what was written.
        if status.is_ok() && stat(dst).map_or(true, |written| u64::from(written.size) != total) {
            let _ = writeln!(Fd(2), "{prog}: error writing '{dst}': the copy is not the size of the original");
            status = Err(Reported);
        }
        if status.is_ok() {
            if let Ok(info) = stat(src) {
                status = self.preserve_from(&info, dst, false);
            }
            self.announce(src, dst);
        }
        status
    }

    /// Copies the directory `src` to `dst` and everything under it. `dst` is created, or merged into if it is already a
    /// directory; a failure on one entry is reported and the rest are still copied, and the whole is `Err(Reported)`.
    pub fn tree(&self, src: &str, dst: &str) -> Result<(), Reported> {
        let prog = self.prog;
        match stat(dst) {
            Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {}
            Ok(_) => {
                let _ = writeln!(Fd(2), "{prog}: cannot overwrite non-directory '{dst}' with directory '{src}'");
                return Err(Reported);
            }
            Err(ENOENT) => {
                let r = mkdir(dst);
                if r < 0 {
                    diag::cannot(prog, "create directory", dst, r);
                    return Err(Reported);
                }
            }
            Err(e) => {
                diag::cannot(prog, "stat", dst, e);
                return Err(Reported);
            }
        }
        self.announce(src, dst);

        let mut entries = match read_dir(src) {
            Ok(entries) => entries,
            Err(e) => {
                diag::report(prog, "error reading", src, e.errno());
                return Err(Reported);
            }
        };
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut result = Ok(());
        for ent in entries {
            let (from, to) = (join(src, &ent.name), join(dst, &ent.name));
            let copied = if ent.attrs & ATTR_DIRECTORY != 0 { self.tree(&from, &to) } else { self.file(&from, &to) };
            if copied.is_err() {
                result = Err(Reported);
            }
        }
        if let Ok(info) = stat(src)
            && self.preserve_from(&info, dst, true).is_err()
        {
            result = Err(Reported);
        }
        result
    }

    /// Copies one operand: a directory with `recursive`, a file otherwise.
    pub fn operand(&self, src: &str, target: &str) -> Result<(), Reported> {
        match stat(src) {
            Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
                if !self.recursive {
                    let _ = writeln!(Fd(2), "{}: -r not specified; omitting directory '{src}'", self.prog);
                    return Err(Reported);
                }
                // A directory cannot be copied into itself.
                let plain = src.trim_end_matches('/');
                if target == plain || target.strip_prefix(plain).is_some_and(|rest| rest.starts_with('/')) {
                    let _ = writeln!(Fd(2), "{}: cannot copy a directory, '{src}', into itself, '{target}'", self.prog);
                    return Err(Reported);
                }
                self.tree(plain, target)
            }
            _ => self.file(src, target),
        }
    }
}

/// Removes `path` and everything under it, contents before their directory (a file is just unlinked). With `report`, each
/// entry that cannot be removed is reported under `prog` (`cannot remove 'x': reason`) and the rest are still tried; without
/// it (cleaning up after a failure, where there is nobody to tell) nothing is said. `Err` if anything was left.
pub fn remove_tree(prog: &str, path: &str, report: bool) -> Result<(), Reported> {
    let fail = |path: &str, code: isize| -> Result<(), Reported> {
        if report {
            diag::cannot(prog, "remove", path, code);
        }
        Err(Reported)
    };
    match stat(path) {
        Ok(info) if info.attrs & ATTR_DIRECTORY != 0 => {
            let entries = match read_dir(path) {
                Ok(entries) => entries,
                Err(e) => return fail(path, e.errno()),
            };
            let mut clean = true;
            for ent in entries {
                clean &= remove_tree(prog, &join(path, &ent.name), report).is_ok();
            }
            // A directory that still has something in it would be refused (`Directory not empty`) on top of the failure
            // already reported for what is in it, so it is left alone.
            if !clean {
                return Err(Reported);
            }
            match unlink(path, true) {
                r if r < 0 => fail(path, r),
                _ => Ok(()),
            }
        }
        Ok(_) => match unlink(path, false) {
            r if r < 0 => fail(path, r),
            _ => Ok(()),
        },
        Err(e) => fail(path, e),
    }
}
