# User programs

The EL0 programs this project builds: each `src/bin/<name>.rs` is its own binary. Most live in one shared Cargo
package, `user/progs/` (built by every stage from Stage 9 on, sharing one `Cargo.toml`, one `link.ld`, and the
helpers in `src/lib.rs`); from Stage 12, `user/progs_r12/` holds this stage's own additions and overrides (a
program that needs the working directory `progs/` predates, or a newer syscall an earlier stage's kernel doesn't
have). `just disk` in a stage's directory builds every tier it uses and stages them into that stage's `disk/bin/`
as `<name>.exe`, lowest tier first, so a later tier's binary of the same name replaces an earlier one's -- the
launcher tries the typed name first and then `name.exe`, so `cat` finds `bin/cat.exe`. This table documents
observable behavior per stage, not which package a program's source happens to live in. (`userlib`,
`user/userlib/`, is the runtime underneath them: entry point, syscall wrappers, `Args`.) Programs are started by the
shell: [`shell.md`](shell.md) describes how a command line (redirections, pipes, scripts) reaches them, and
[`launching_programs.md`](launching_programs.md) how one is loaded and started. What a program can ask the kernel for is in [`syscalls.md`](syscalls.md).

## Conventions

Every program is a deliberate *subset* of its POSIX namesake, in the same spirit as the rest of the project: the
Cygwin approach from `ROADMAP.md`'s intro, matching POSIX styling wherever it's cheap and being explicit where it isn't.
The table below is the record of exactly which subset. Anything not listed as supported is unsupported; an unknown
option prints `<prog>: unknown option: <arg>` and exits 1.

- **Errors** go to stderr as `<prog>: <arg>: <reason>` -- both stdout and stderr reach the console (and the serial log)
  -- and the program exits 1. Success is exit 0. A nonzero status is reported by the launcher as `exit N`; a program
  stopped by a fault is reported as `exit 139`.
- **`--help`** prints a usage synopsis and, for programs that take flags, one line per flag, then exits 0 -- every
  program has it (except `hello`/`crash`, Stage 9's frozen test programs), so it isn't repeated per-row below.
  `-h` is deliberately not an alias: POSIX has no help-flag convention for any of these utilities at all, and GNU
  coreutils' own `--help` is long-form only in every one of them too, precisely because `-h` already means
  something else in some (`ls -h`/`du -h` human-readable sizes, `cp -h`/`chmod -h` no-dereference) -- `--help` is
  the one form every program here guarantees, for the same reason.
- **Paths** are absolute or relative to the working directory (the root before Stage 12's shell, which introduced `cd`);
  from Stage 12 `.` and `..` are resolved lexically (`a/../b` is `b`, and `..` at the root stays there). Names match
  case-sensitively.
- **A lone `-`** is an ordinary file name, not "stdin", wherever a file operand is taken.
- **stdin** is the keyboard unless the shell redirected it (`<`, or a pipe, from Stage 12): a read returns one finished
  line at a time, with Backspace and Ctrl+U (discard the line) already applied. Ctrl+D on an empty line is end-of-file;
  `From Stage 12:` on a non-empty line it delivers what has been typed so far *without* a newline. Cursor keys, history
  and the other editing keys belong to the shell's prompt, not to a program's input (see [`console.md`](console.md)).
- **Adding to this table:** a new program gets a row, with the stage that introduced it. When a later stage gives an
  existing program a new feature, that feature is written in the *Supported* column as `From Stage N: <feature>` and
  removed from *Not supported*, so this stays a per-feature history.

## Programs

| Program | Stage | POSIX equivalent | Supported | Deliberately not supported |
|---|---|---|---|---|
| `hello`, `crash` | 9 | -- (test programs) | `hello` prints a line and exits; `crash` reads an invalid address to demonstrate the fault path | -- |
| `echo` | 10 | `echo [-n] args...` | arguments joined by spaces, plus a newline; `From Stage 12: -n` suppresses it | `-e` and escape sequences (an argument other than a leading `-n`, `-e` included, is printed as ordinary text, not rejected) |
| `cat` | 11 | `cat [file...]` | zero or more files, concatenated in order; no files = stdin until EOF | `-u` and every GNU flag (`-n`, `-A`, ...) |
| `ls` | 11 | `ls -1 [-F] [-l] [dir...]` | one entry per line, always (`-1`; no columns); `-F` appends `/` to directories and `*` to files with the executable bit; `dir` defaults to the working directory (`From Stage 12`; root before it); `From Stage 12: -l` long format (`d`/`w`/`x` flags -- directory, writable [FAT's read-only bit inverted, matching Unix's positive-capability convention], executable -- then size, name); `From Stage 12:` several directory operands, each preceded by a `name:` header line (with a blank line between listings) when more than one is given | `-a`/`-R`/`-d` and sorting options; `.`/`..` entries; entries appear in on-disk order, not sorted |
| `cp` | 11 | `cp SRC... DST` | two or more operands; creates `DST` if absent, replaces its contents if present (a read-only `DST` is refused, and an unreadable `SRC` leaves `DST` untouched); `From Stage 12:` several sources, and a directory as `DST` (each source lands at `DST/basename(SRC)`); `From Stage 12:` a source whose path is textually identical to its destination (`cp a a`) is refused (`Invalid argument`) -- other spellings of the same file aren't detected | `-r`/`-f`/`-p`/`-i`; preserving attributes or timestamps |
| `head` | 11 | `head [-n N \| -c N] [file]` | `-n N` (default 10); `From Stage 12: -c N` (byte count, mutually exclusive with `-n`); a file, or stdin | the `-N` shorthand; several files and their `==> name <==` headers |
| `tail` | 11 | `tail [-n N \| -c N] [file]` | `-n N` (default 10); `From Stage 12: -c N` (byte count, mutually exclusive with `-n`); a named file (read twice, so any size) or stdin (buffered whole, up to 512 KiB) | `-f`; the `+N` form; several files |
| `wc` | 11 | `wc [-l] [-w] [-c] [-L] [file...]` | any combination of `-l`/`-w`/`-c` (none of `-l`/`-w`/`-c`/`-L` = all three), counts in POSIX's order (lines, words, bytes) separated by single spaces; `From Stage 12: -L` (longest line); `From Stage 12:` several files, each on its own line, plus a `total` line when more than one is given; a file, or stdin | `-m`; locale-aware counting (bytes only) |
| `hexdump` | 11 | `hexdump [file]`, producing `hexdump -C`'s output (BSD/util-linux; not POSIX, which has only `od`) | the canonical format only, always (it takes no options, so `-C` itself is refused): offset, 16 hex bytes, `\|ASCII\|`, and a closing offset line; a file, or stdin | every flag, including `-C`, and custom `-e` formats; collapsing repeated rows into `*` (every row is printed) |
| `true` | 11 | `true` | exits 0 (any argument besides `--help` is ignored) | -- |
| `false` | 11 | `false` | exits 1 (any argument besides `--help` is ignored) | -- |
| `chmod` | 11 | `chmod +x\|-x\|+w\|-w [-R] file...` (POSIX symbolic mode with `who` omitted, which POSIX defines as "all"; this is a single-user system) | `+x`/`-x` set/clear the executable bit (`0x40`, this project's own convention -- see `ROADMAP.md`'s Stage 8); `+w`/`-w` clear/set FAT's read-only bit; `From Stage 12:` several files, and `-R` (recurses into directory operands, depth-first) | an explicit `who` (`u`/`g`/`o`/`a`), since there are no classes to tell apart; octal modes; `r` (FAT has no read bit); `s`/`t`/`X` |
| `pwd` | 12 | `pwd` (POSIX also defines `-L`/`-P`, which of the logical and physical path to print) | prints the working directory (absolute), via the `getcwd` syscall | `-L`/`-P`, refused with a clear error -- there are no symbolic links to tell them apart |
| `clear` | 12 | `clear` (not POSIX; conventionally from `ncurses`/`terminfo`) | clears the console and homes the cursor, via the `ioctl` syscall; fails with `Inappropriate ioctl for device` if stdout isn't the console | -- |
| `mkdir` | 12 | `mkdir DIR...` | one or more operands, continuing past a failing one (status 1 if any failed); `DIR`'s parent must already exist, `DIR` itself must not | `-p`, `-m` |
| `rm` | 12 | `rm [-r] [-f] PATH...` | one or more operands, continuing past a failing one; plain `rm` refuses a directory (`Is a directory`) and refuses `.`, `..`, and `/` outright (`-f` never overrides this guard, matching GNU); `-r` drills into a directory before removing it; `-f` skips a missing operand silently instead of reporting it. Deletion is never gated by the target's own read-only bit -- unlike a real prompt-before-overwrite tty session, this project has no non-root identity, and real POSIX conditions that prompt (and `-f`'s suppression of it) on the process *not* having appropriate privileges: deletion is governed by the containing directory, never a file's own mode, so a privileged process was never blocked here either | `-i`; a separate `rmdir` |
| `mv` | 12 | `mv SRC... DST` | renames within the volume; if `DST` is an existing directory the source moves into it (`DST/basename(SRC)`); if `DST` is an existing plain file it is replaced (source and destination must both be plain files -- replacing across a directory on either side is refused, `File exists`, since a partial replace could orphan a directory's contents); a directory can't be moved into itself or a descendant (`Invalid argument`); `mv a a` is refused; a trailing `/` on `DST` requires an existing directory; more than one source requires an existing directory destination | `-f`, `-i`; attributes and the exec/read-only bits move with the entry automatically (`rename` copies the whole directory entry) |
| `stat` | 12 | (no POSIX equivalent; the shape below matches common `stat(1)` implementations, scoped to what FAT actually stores) | one or more operands: size, type (regular file/directory), the two tracked attribute bits, and all three real FAT timestamps at their native 2-second resolution (creation, last modified, and last accessed, which FAT stores as a date only -- no time component) | anything not stored by FAT: inode, hard-link count (FAT has none), uid/gid/full permission bits (no user model), symlinks (none exist). Every timestamp reads as whatever's actually stored -- the fixed build-time stamp for a file bundled with the image, or the FAT epoch (1980-01-01) for anything the kernel itself created or wrote this session, since no RTC exists until Stage 14 |
| `tee` | 12 | `tee [-a] [file...]` | copies stdin to stdout and to zero or more named files at once (POSIX allows zero, an odd but valid way to copy stdin to stdout); `-a` appends to each file instead of truncating it (it applies to files named after it: `tee f -a` still truncates `f`); a file that fails to open is reported and skipped, stdin still passed through and every other file still written; up to 8 files held open at once | `-i` (ignore `SIGINT`; no signal handling exists at all yet) |
| `poweroff` | 12 | `poweroff` (`poweroff(8)`; not POSIX) | powers off the machine via PSCI `SYSTEM_OFF`, reached through the kernel's `reboot` syscall; `--reboot` restarts it instead (PSCI `SYSTEM_RESET`) | scheduling (`shutdown`'s mandatory `TIME` operand) -- there is no RTC until Stage 14, so every action is immediate; `-f`/`-n`/`-w`/wall messages |
| `reboot` | 12 | `reboot` (`reboot(8)`; not POSIX) | restarts the machine via PSCI `SYSTEM_RESET` -- equivalent to `poweroff --reboot` | `-f`/`-n`/`-w` |

One footgun, same as on Linux: `chmod -x bin/chmod.exe` locks `chmod` out of running until the bit is restored from the
host (`disk.img` can be rebuilt with `just disk`).

## Testing

Tests for these programs live in the stage that builds them: in `r12_shell/test/cases/`, `core_utils.py` (the Stage
9-11 utilities and `tee`, the regression baseline) and `user_progs.py` (Stage 12's additions), driven through the QEMU harness
described in [`tests.md`](tests.md). New programs get cases there. (`r11_busybox/` keeps its own, older suite.)
