# User programs

The EL0 programs this project builds, all from one Cargo package, `user/progs/`: each `src/bin/<name>.rs` is its own
binary, sharing one `Cargo.toml`, one `link.ld`, and the helpers in `src/lib.rs`. `just disk` in a stage's directory
builds them all once and stages them into that stage's `disk/bin/` as `<name>.exe` -- the launcher tries the typed name
first and then `name.exe`, so `cat` finds `bin/cat.exe`. (`userlib`, `user/userlib/`, is the runtime underneath them:
entry point, syscall wrappers, `Args`.)

## Conventions

Every program is a deliberate *subset* of its POSIX namesake, in the same spirit as the rest of the project: the
Cygwin approach from `ROADMAP.md`'s intro, matching POSIX styling wherever it's cheap and being explicit where it isn't.
The table below is the record of exactly which subset. Anything not listed as supported is unsupported; an unknown
option prints `<prog>: unknown option: <arg>` and exits 1.

- **Errors** go to stderr as `<prog>: <arg>: <reason>` -- both stdout and stderr reach the console (and the serial log)
  -- and the program exits 1. Success is exit 0. A nonzero status is reported by the launcher as `exit N`; a program
  stopped by a fault is reported as `exit 139`.
- **Paths** are absolute or relative to the root -- there is no working directory until Stage 12's shell. Names match
  case-sensitively; `.` and `..` aren't special.
- **A lone `-`** is an ordinary file name, not "stdin", wherever a file operand is taken.
- **stdin** is the keyboard: a read returns one finished line at a time (Backspace already applied), and Ctrl+D on an
  empty line is end-of-file.
- **Adding to this table:** a new program gets a row, with the stage that introduced it. When a later stage gives an
  existing program a new feature, that feature is written in the *Supported* column as `From Stage N: <feature>` and
  removed from *Not supported*, so this stays a per-feature history.

## Programs

| Program | Stage | POSIX equivalent | Supported | Deliberately not supported |
|---|---|---|---|---|
| `hello`, `crash` | 9 | -- (test programs) | `hello` prints a line and exits; `crash` reads an invalid address to demonstrate the fault path | -- |
| `echo` | 10 | `echo` | arguments joined by spaces, plus a newline | `-n`, `-e`, escape sequences |
| `cat` | 11 | `cat [file...]` | zero or more files, concatenated in order; no files = stdin until EOF | `-u` and every GNU flag (`-n`, `-A`, ...) |
| `ls` | 11 | `ls -1 [-F] [dir]` | one entry per line, always (`-1`; no columns); `-F` appends `/` to directories and `*` to files with the executable bit; `dir` defaults to `/` | `-a`/`-l`/`-R`/`-d` and sorting options; more than one operand; a file operand (`dir` must be a directory); `.`/`..` entries; entries appear in on-disk order, not sorted |
| `cp` | 11 | `cp src dst` | exactly two regular-file operands; creates `dst` if absent, replaces its contents if present (a read-only `dst` is refused, and an unreadable `src` leaves `dst` untouched) | `-r`/`-f`/`-p`/`-i`; a directory as `dst` (`cp f dir/`); several sources; preserving attributes or timestamps |
| `head` | 11 | `head [-n N] [file]` | `-n N` (default 10); a file, or stdin | `-c`; the `-N` shorthand; several files and their `==> name <==` headers |
| `tail` | 11 | `tail [-n N] [file]` | `-n N` (default 10); a named file (read twice, so any size) or stdin (buffered whole, up to 512 KiB) | `-f`; `-c`; the `+N` form; several files |
| `wc` | 11 | `wc [-l] [-w] [-c] [file]` | any combination of `-l`/`-w`/`-c` (none = all three), counts in POSIX's order (lines, words, bytes) separated by single spaces; a file, or stdin | `-m` and `-L`; several files and a totals line; locale-aware counting (bytes only) |
| `hexdump` | 11 | `hexdump -C [file]` (BSD/util-linux; not POSIX, which has only `od`) | the canonical format only: offset, 16 hex bytes, `\|ASCII\|`, and a closing offset line; a file, or stdin | every other flag and custom `-e` formats; collapsing repeated rows into `*` (every row is printed) |
| `true` | 11 | `true` | exits 0 | -- |
| `false` | 11 | `false` | exits 1 | -- |
| `chmod` | 11 | `chmod +x\|-x\|+w\|-w file` (POSIX symbolic mode with `who` omitted, which POSIX defines as "all"; this is a single-user system) | `+x`/`-x` set/clear the executable bit (`0x40`, this project's own convention -- see `ROADMAP.md`'s Stage 8); `+w`/`-w` clear/set FAT's read-only bit; one file | an explicit `who` (`u`/`g`/`o`/`a`), since there are no classes to tell apart; octal modes; `r` (FAT has no read bit); `s`/`t`/`X`; `-R`; several files |

One footgun, same as on Linux: `chmod -x bin/chmod.exe` locks `chmod` out of running until the bit is restored from the
host (`disk.img` can be rebuilt with `just disk`).

## Testing

`just test` in `r11_busybox/` boots the kernel headless and drives it by typing on the virtio keyboard through the
QEMU monitor's `sendkey`, checking each command's output against the serial log and the resulting disk contents (see
`r11_busybox/test/run_tests.py`). New programs get cases there.
