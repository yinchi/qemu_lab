# User programs

The EL0 programs this project builds: each `src/bin/<name>.rs` is its own binary. Most live in one shared Cargo
package, `user/progs/` (built by every stage from Stage 9 on, sharing one `Cargo.toml`, one `link.ld`, and the
helpers in `src/lib.rs`); from Stage 12, `user/progs_r12/` holds this stage's own additions and overrides (a
program that needs the working directory `progs/` predates, or a newer syscall an earlier stage's kernel doesn't
have). `just disk` in a stage's directory builds every tier it uses and stages them into that stage's `disk/bin/`
as `<name>.exe`, lowest tier first, so a later tier's binary of the same name replaces an earlier one's -- the
launcher tries the typed name first and then `name.exe`, so `cat` finds `bin/cat.exe`. This table documents
observable behavior per stage, not which package a program's source happens to live in. (`userlib`,
`user/userlib/`, is the runtime underneath them: entry point, syscall wrappers, `Args`, and from Stage 17 `env::var`/`env::vars` for a program started with `entry_with_env!`.) From Stage 17, `user/progs_r17/` holds `env` and `printenv`. Programs are started by the
shell: [`shell.md`](shell.md) describes how a command line (redirections, pipes, scripts) reaches them, and
[`launching_programs.md`](launching_programs.md) how one is loaded and started. What a program can ask the kernel for is in [`syscalls.md`](syscalls.md).

## Conventions

Every program is a deliberate *subset* of its POSIX namesake, in the same spirit as the rest of the project: the
Cygwin approach from `ROADMAP.md`'s intro, matching POSIX styling wherever it's cheap and being explicit where it isn't.
The table below is the record of exactly which subset. Anything not listed as supported is unsupported; an unknown
option is refused with a message and exit 1 (worded as under "Errors").

- **Errors** go to stderr, both stdout and stderr reach the console (and the serial log), and the program exits 1.
  Success is exit 0. A nonzero status is visible as `$?` (the shell prints nothing itself, from Stage 17; before
  that the launcher printed `exit N`); a program stopped by a fault is 139. The wording is GNU coreutils' (in the C locale, ASCII quotes), from Stage 12 on:
  - a failed operation is `<prog>: cannot <verb> '<path>': <reason>` (`cannot remove`, `cannot stat`, `cannot create
    directory`, `cannot create regular file`, `cannot move 'a' to 'b'`, `cannot access`, `cannot open ... for
    reading`), or `error reading`/`error writing '<path>'`, `reading directory`, `changing permissions of` where GNU
    says so; `cat`, `wc` and `tee` keep GNU's bare `<prog>: <path>: <reason>`;
  - a bad option is `invalid option -- 'x'` (or `unrecognized option '--foo'`), a missing or extra operand is
    `missing operand`, `missing file operand`, `missing destination file operand after 'a'`, `extra operand 'x'`
    or `target 'd' is not a directory`, and each of these is followed by `Try '<prog> --help' for more
    information.`; `head`/`tail` say `invalid number of lines: 'x'` (or `bytes`) and `option requires an argument
    -- 'n'`; `chmod` says `invalid mode: 'x'` (only the four symbolic modes exist, so an octal `755` is invalid here);
  - refusals: `cp`/`mv` `'a' and 'a' are the same file`, `mv` `cannot move 'd' to a subdirectory of itself, 'd/x'`,
    `cp` `-r not specified; omitting directory 'd'`, and `rm` `refusing to remove '.' or '..' directory: skipping
    '.'`, `cannot remove '/': Is a directory` and `it is dangerous to operate recursively on '/'`.

  These come from `progs::diag` (`user/progs/src/diag.rs`), used only by the Stage 12 tier. The base tier's programs
  (`hello`, `crash`, `echo`, `true`, `false` and, in the Stage 9-11 kernels, `cat`, `wc`, `hexdump`, `cp`, `head`,
  `tail`, `ls`, `chmod`) keep `<prog>: unknown option: <arg>`, `usage: ...` and bare `<prog>: <arg>: <reason>`,
  which r09-r11's tests pin; the Stage 12 tier replaces the ones that had something to change, by name.
- **Options** are parsed by one crate, `getargs` (`no_std`, no allocation), through `progs_r12::cli`, for every Stage
  12 program: short flags may be grouped (`-Fl`), an option's value may be attached or separate (`-n5`, `-n 5`,
  `--lines=5`, `--lines 5`), `--` ends the options, a lone `-` is an operand (a file named `-`), and options may
  come after the operands (`rm dir -r`, `tee file -a`): a flag applies to the whole command line. `head`/`tail` also
  take `--lines` and `--bytes`, and `tee` `--append`; every other long option is refused. `chmod` is the exception
  in how it reads its first operand: the modes `-x` and `-w` look like options, so it treats only `-R`, `--help` and
  `--` as options (`chmod -- -x file` works). The base tier keeps the older per-program loops, which r09-r11 use.
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
| `ls` | 11 | `ls -1 [-F] [-l] [dir...]` | one entry per line, always (`-1`, which is also accepted as a flag and changes nothing; no columns); `From Stage 12:` short flags may be grouped (`-Fl`); `-F` appends `/` to directories and `*` to files with the executable bit; `dir` defaults to the working directory (`From Stage 12`; root before it); `From Stage 12: -l` long format (`d`/`w`/`x` flags -- directory, writable [FAT's read-only bit inverted, matching Unix's positive-capability convention], executable -- then size, name); `From Stage 12:` several directory operands, each preceded by a `name:` header line (with a blank line between listings) when more than one is given | `-a`/`-R`/`-d` and sorting options; `.`/`..` entries; `From Stage 16:` the entries of each directory are **sorted by name**, bytewise (the C locale's order: `Mid` before `alpha`); Stages 11-15 printed them in on-disk order, not sorted |
| `cp` | 11 | `cp SRC... DST` | two or more operands; creates `DST` if absent, replaces its contents if present (a read-only `DST` is refused, and an unreadable `SRC` leaves `DST` untouched); `From Stage 12:` several sources, and a directory as `DST` (each source lands at `DST/basename(SRC)`); `From Stage 12:` a source whose path is textually identical to its destination (`cp a a`) is refused (`cp: 'a' and 'a' are the same file`) -- other spellings of the same file aren't detected | `-r`/`-f`/`-p`/`-i`; preserving attributes or timestamps |
| `head` | 11 | `head [-n N \| -c N] [file]` | `-n N` (default 10); `From Stage 12: -c N` (byte count, mutually exclusive with `-n`); a file, or stdin | the `-N` shorthand; several files and their `==> name <==` headers |
| `tail` | 11 | `tail [-n N \| -c N] [file]` | `-n N` (default 10); `From Stage 12: -c N` (byte count, mutually exclusive with `-n`); a named file (read twice, so any size) or stdin, which is read once into a growable buffer that keeps only the last N lines or bytes, so any amount of input works (`From Stage 16`; Stages 12-15 buffered stdin whole, in a fixed 512 KiB, and refused more) | `-f`; the `+N` form; several files |
| `wc` | 11 | `wc [-l] [-w] [-c] [-L] [file...]` | any combination of `-l`/`-w`/`-c` (none of `-l`/`-w`/`-c`/`-L` = all three), counts in POSIX's order (lines, words, bytes) separated by single spaces; `From Stage 12: -L` (longest line); `From Stage 12:` several files, each on its own line, plus a `total` line when more than one is given; a file, or stdin | `-m`; locale-aware counting (bytes only) |
| `hexdump` | 11 | `hexdump [file]`, producing `hexdump -C`'s output (BSD/util-linux; not POSIX, which has only `od`) | the canonical format only, always (it takes no options, so `-C` itself is refused): offset, 16 hex bytes, `\|ASCII\|`, and a closing offset line; a file, or stdin | every flag, including `-C`, and custom `-e` formats; collapsing repeated rows into `*` (every row is printed) |
| `true` | 11 | `true` | exits 0 (any argument besides `--help` is ignored) | -- |
| `false` | 11 | `false` | exits 1 (any argument besides `--help` is ignored) | -- |
| `chmod` | 11 | `chmod +x\|-x\|+w\|-w [-R] file...` (POSIX symbolic mode with `who` omitted, which POSIX defines as "all"; this is a single-user system) | `+x`/`-x` set/clear the executable bit (`0x40`, this project's own convention -- see `ROADMAP.md`'s Stage 8); `+w`/`-w` clear/set FAT's read-only bit; `From Stage 12:` several files, and `-R` (recurses into directory operands, depth-first; `From Stage 16:` to any depth -- each directory is listed whole and closed before recursing, where Stages 12-15 kept every ancestor open and gave out about ten levels down) | an explicit `who` (`u`/`g`/`o`/`a`), since there are no classes to tell apart; octal modes; `r` (FAT has no read bit); `s`/`t`/`X` |
| `pwd` | 12 | `pwd` (POSIX also defines `-L`/`-P`, which of the logical and physical path to print) | prints the working directory (absolute), via the `getcwd` syscall | `-L`/`-P`, refused with a clear error -- there are no symbolic links to tell them apart |
| `clear` | 12 | `clear` (not POSIX; conventionally from `ncurses`/`terminfo`) | clears the console and homes the cursor, via the `ioctl` syscall; fails with `Inappropriate ioctl for device` if stdout isn't the console | -- |
| `mkdir` | 12 | `mkdir DIR...` | one or more operands, continuing past a failing one (status 1 if any failed); `DIR`'s parent must already exist, `DIR` itself must not | `-p`, `-m` |
| `rm` | 12 | `rm [-r] [-f] PATH...` | one or more operands, continuing past a failing one; plain `rm` refuses a directory (`Is a directory`) and refuses `.`, `..`, and `/` outright (`-f` never overrides this guard, matching GNU); `-r` drills into a directory before removing it (`From Stage 16:` listing each directory whole, where Stages 12-15 reopened it once per entry); `-f` skips a missing operand silently instead of reporting it. Deletion is never gated by the target's own read-only bit -- unlike a real prompt-before-overwrite tty session, this project has no non-root identity, and real POSIX conditions that prompt (and `-f`'s suppression of it) on the process *not* having appropriate privileges: deletion is governed by the containing directory, never a file's own mode, so a privileged process was never blocked here either | `-i`; a separate `rmdir` |
| `mv` | 12 | `mv SRC... DST` | renames within the volume; if `DST` is an existing directory the source moves into it (`DST/basename(SRC)`); if `DST` is an existing plain file it is replaced (source and destination must both be plain files -- replacing across a directory on either side is refused, `File exists`, since a partial replace could orphan a directory's contents); a directory can't be moved into itself or a descendant (`mv: cannot move 'd' to a subdirectory of itself, 'd/x'`); `mv a a` is refused (`mv: 'a' and 'a' are the same file`); a trailing `/` on `DST` requires an existing directory; more than one source requires an existing directory destination | `-f`, `-i`; attributes and the exec/read-only bits move with the entry automatically (`rename` copies the whole directory entry) |
| `stat` | 12 | (no POSIX equivalent; the shape below matches common `stat(1)` implementations, scoped to what FAT actually stores) | one or more operands: size, type (regular file/directory), the two tracked attribute bits, and all three real FAT timestamps at their native 2-second resolution (creation, last modified, and last accessed, which FAT stores as a date only -- no time component) | anything not stored by FAT: inode, hard-link count (FAT has none), uid/gid/full permission bits (no user model), symlinks (none exist). Every timestamp reads as whatever's actually stored, and what is stored is UTC: the fixed build-time stamp for a file bundled with the image, and the real-time clock's time (Stage 14) for anything created or written since -- unlike Stages 9-13, which left the FAT epoch (1980-01-01) on everything the kernel wrote. Stages 12-14 print the stored UTC fields with no zone shown. `From Stage 15:` `stat` converts the created and modified times to the local zone (`America/Toronto`, from `chrono-tz`, hard-coded until Stage 17's `$TZ`) and shows its abbreviation -- `2001-09-08 21:46:40 EDT`, the same as `date` -- and **no longer shows the accessed date**: FAT stores only a date for it, nothing here updates it on a read (see [`filesystem.md`](filesystem.md): the same as Linux's `noatime`), and it is set only when an entry is created or written, to the modified date -- so it always repeated the `Modify` line, as a UTC date beside a local time that can fall on another day. The syscall still returns it. Stages 12-14's `stat` print an `Access:` line The FAT epoch that the root reports is therefore `1979-12-31 19:00:00 EST` (in Stages 12-14, `1980-01-01 00:00:00`) |
| `tee` | 12 | `tee [-a] [file...]` | copies stdin to stdout and to zero or more named files at once (POSIX allows zero, an odd but valid way to copy stdin to stdout); `-a` appends to each file instead of truncating it (it applies to every file, wherever it is written); a file that fails to open is reported and skipped, stdin still passed through and every other file still written; `From Stage 16:` as many files as the kernel lets a program hold open (13 at a time, fewer in a pipeline, which holds the pipe's temp file), a file that does not open being reported `Too many open files` and skipped; Stages 12-15 held at most 8 | `-i` (ignore `SIGINT`; no signal handling exists at all yet) |
| `poweroff` | 12 | `poweroff` (`poweroff(8)`; not POSIX) | powers off the machine via PSCI `SYSTEM_OFF`, reached through the kernel's `reboot` syscall; `--reboot` restarts it instead (PSCI `SYSTEM_RESET`) | scheduling (`shutdown`'s mandatory `TIME` operand) -- nothing here schedules anything by the clock (the RTC exists as of Stage 13, but a later shutdown would need a timer that acts on it), so every action is immediate; `-f`/`-n`/`-w`/wall messages |
| `reboot` | 12 | `reboot` (`reboot(8)`; not POSIX) | restarts the machine via PSCI `SYSTEM_RESET` -- equivalent to `poweroff --reboot` | `-f`/`-n`/`-w` |
| `date` | 13 | `date [-u] [-d @SECONDS] [-I[FMT] \| -R \| +FORMAT]` | prints the time from the real-time clock in GNU's default layout (`Sat Sep  8 21:46:40 EDT 2001`); `+FORMAT` takes `chrono`'s `strftime` conversions (`%Y %m %d %H %M %S %s %a %A %b %B %e %j %F %T %D %R %p %u %w %U %W %G %V %Z %z ...`, see its documentation) with the flags `-` (no padding), `_` (space) and `0`; a conversion it does not know is an error, `date: invalid format '+...'`, where GNU would print it as written; `-I[date\|hours\|minutes\|seconds]` (ISO 8601, with the real offset: `-04:00`) and `-R` (RFC 5322); `-d @SECONDS` prints that instant instead of now (negative allowed). `From Stage 15:` **the time is in `America/Toronto`** (hard-coded until Stage 17's `$TZ`), with the zone's real rules -- `EST`/`EDT`, `-0500`/`-0400`, the daylight-saving changes -- from the whole IANA database, which `chrono-tz` builds into the program (about 1.3 MB: the reason Stage 15's larger program window exists), and `-u`/`--utc` prints UTC. Stage 13 (and 14) `date` printed UTC and accepted `-u` as a no-op | setting the time (`-s`, or a `MMDDhhmm` operand) and reading it from a string other than `@SECONDS` (`-d tomorrow`); choosing a zone until Stage 17; the `^` flag and locale names; sub-second output (`%N`, `-Ins`): the PL031 counts whole seconds; the clock's own range is 1970 to 2106 |
| `env` | 17 | `env` (POSIX and GNU also run a command in a changed environment: `env NAME=VALUE cmd`, `-i`, `-u`) | prints the environment the shell gave it, one `NAME=VALUE` line per exported variable, in the order they were first set | every option and operand (`env: extra operand 'x'`): the shell's own `NAME=VALUE cmd` (Stage 17) is how a command runs in a changed environment |
| `printenv` | 17 | `printenv [NAME]...` (GNU coreutils; not POSIX) | with no operand, the whole environment like `env`; otherwise each named variable's value on a line of its own, and exit status 1 if any name is not set (which prints nothing for it) | options; a `NAME` containing `=` is simply never set |

One footgun, same as on Linux: `chmod -x bin/chmod.exe` locks `chmod` out of running until the bit is restored from the
host (`disk.img` can be rebuilt with `just disk`).

## Testing

Tests for these programs live in the stage that builds them: in `r16_brk/test/cases/`, `core_utils.py` (the Stage
9-11 utilities and `tee`, the regression baseline) and `user_progs.py` (Stage 12's additions), driven through the QEMU harness
described in [`tests.md`](tests.md). New programs get cases there. (`r11_busybox/` keeps its own, older suite.)
