# Stage 18: more utilities -- `r18_utils` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the steps in the order they are built and committed. It is updated as each step lands (an "As built" note per step, as
`Stage17.md` does). Programs only: **no kernel or shell change**, a new tier `user/progs_r18` on the scheme every stage since 12 uses (the highest tier wins a name).

| Step | What | Status |
|---|---|---|
| R | Insert the stage in the roadmap (Stage 18; the storage stage becomes 19, the editor 20, job control 21-26) | done |
| 0 | Plain copy of `r17_env` as `r18_utils` | done |
| 1 | The tier `progs_r18`; `rmdir`, `touch`, `seq`, `cmp` | done |
| 2 | Flag catch-up: `mkdir -p -v`, `cp -r -n -v`, `mv -n -v -f`, `rm -v -d`, `ls -a -d -R -r -t -S -h` (dotfiles hidden by default) | done |
| 3 | Filters: `sort`, `uniq`, `cut`, `tr`, `find`, `fgrep`; pure helpers host-tested | done |
| 4 | Text-tool flags: `cat -n -E -T -s`, `head`/`tail` several files and `-n -N`/`-n +N`, `wc -m`, `echo -e -E` | done |
| 5 | Docs, roadmap "As built", regression sweep | planned |

## Decisions (settled)
- **Its own stage, before persistent storage.** Programs and a tier are a stage (`r11_busybox` is the precedent). Storage (now Stage 19) then extends the `mv` this stage puts in the tier with a cross-volume fallback.
- **New small programs:** `rmdir`, `touch`, `seq`, `cmp`. `touch` refreshes an existing file's modify time by opening it for append and closing (hadris' `FileWriter::finish` stamps the entry with the clock; an append writer
  keeps the data). That is read from the source, not yet run: **the first test of Step 1 checks it, and `touch` is dropped from the stage if it fails** (the user's decision), not shipped create-only.
- **`ls` hides names starting with `.` unless `-a`;** the kernel never returns `.` and `..`, so `-a` adds only dot-named files. A file operand is listed as itself.
- **Filters:** `sort`, `uniq`, `cut`, `tr`, `find`, and `fgrep` (fixed strings) only: **no `grep`** until a `no_std` regex crate is checked.
- **Documentation stays stage-agnostic** (`<stage>` for what changes per stage; name the stage for a change attributable to one): new rows say "From Stage 18".

## Not doing
`yes` (until Ctrl+C, Stage 22, and streaming pipes, Stage 25), `whoami`/`uname`/`hostname`/`id`, `basename`/`dirname`/`realpath` (need `$(...)`), `cksum`/`nl`/`tac`/`rev`, `grep` with regexes, `sed`/`awk`/`diff`/`printf`/`dd`,
`sort -k`, `find -exec`, `ls -l` with a time column, `cp -p`, `touch -d/-t/-r`, `sleep`/`xargs`/`time`/`env CMD`/`kill`.

## Step 0 -- plain copy `rust/r18_utils`
`rsync` `r17_env` -> `r18_utils` (excluding `target`, `disk.img`, `*.elf`, `__pycache__`), rename `r17_env` in `Cargo.toml`/`Cargo.lock`, `justfile` `BIN`, `test/check_docs.py`, `test/run_tests.py` (temp-dir prefix),
`test/README.md`, `hosttests/src/lib.rs`, `disk/tests/notes.txt`, `disk/fonts/NOTICE` if they name it, and the stage's own `.gitignore`. `just test` = 1083 checks, nothing else changed.

**As built (Step 0).** `rsync` excluded `target`, `*.elf`, `disk.img`, `__pycache__` and the generated files (`disk/bin`, `disk/tmp`, the test programs and malformed-ELF fixtures, `biglines.txt`, `bigenv.sh`); the stage's own `.gitignore` came with it. `r17_env` was renamed in
`Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (docstring and the `r18-` temp prefix), `test/README.md`, `disk/tests/notes.txt` and `disk/fonts/NOTICE`. The tier list is unchanged.
`just test` = 1083 checks (and 217 host tests), `just lint` and `just check-docs` clean: identical to Stage 17.

## Step 1 -- the tier and four small programs
New crate `user/progs_r18` (copy of `progs_r17`'s `Cargo.toml`, `build.rs`, `.cargo/config.toml`: `userlib` with `heap`, `progs`, `progs_r12`, `getargs`); the tier goes in `r18_utils/justfile`'s `for tier in ...` and `lint` lists.
- `rmdir DIR...`: `AT_REMOVEDIR` unlink, GNU wording (`rmdir: failed to remove 'd': Directory not empty`); the kernel already returns `ENOTEMPTY`/`ENOTDIR`.
- `touch [-c] FILE...`: `open(O_WRONLY|O_APPEND)` + `close`; `-c` does not create; a directory is `cannot touch 'd': Is a directory`. Test first (modify time before < after on a non-empty file; data and size unchanged).
- `seq [-s SEP] [-w] [FIRST [INCR]] LAST` (i64, INCR 0 refused); `cmp [-s] FILE1 FILE2` (`-` is stdin; status 0/1/2; `f1 f2 differ: byte N, line M`, `cmp: EOF on f1 after byte N`).
- Tests: group `tools`. Docs: four rows in `docs/progs.md`, `tests.md`.

**As built (Step 1).** As planned, with these specifics:
- **`touch` works** -- the conditional in the plan is settled: an append-mode open and close of a file that already has data moves its modify time to now and changes nothing else (size, creation time, data and the exec bit are checked). The test
  waits 3.2 s of real time between the two `stat`s (FAT time has a 2-second tick; the guest's clock follows the host's) and runs with `TZ=UTC` so the stamps compare as text.
- `seq` reads its arguments by hand (not `getargs`): `-5` is a number there, not an option. Integers only (i64, stops at the end of the range instead of wrapping); `-w` counts the sign in the width. `cmp` exits 2 for every kind of trouble,
  usage errors included (GNU's convention), and its EOF message is the short form (`cmp: EOF on f1 after byte N`, no line). `rmdir` and `touch` take their wording from `progs::diag` (`failed to remove`, `cannot touch`).
- New crate `user/progs_r18` (empty `lib.rs` for now; the filters' pure helpers land there in Step 3) and the tier added to `r18_utils/justfile`. Tests: group `tools` (79 checks); rows for the four programs in `docs/progs.md` (`just check-docs`: 29 programs).
  `just test` = 1162 checks.
- **The flake, and what was done about it.** `token_queue`'s "typing during a large copy" check (real key presses racing a 3 MiB `cp`) lost a key in two of three full runs at 6 and 8 parallel workers (`intct` for `intact`); at
  `QEMU_TEST_WORKERS=4` the suite passed. `run_tests.py` now has an **`EXCLUSIVE`** module attribute: such a group (today `token_queue`) is taken out of the pool and run by itself after every other group has finished, so the rest keep
  the full parallelism (default: CPUs minus one) and this check no longer competes with other sessions. **That does not cure it:** run alone on a quiet machine the check still fails about one time in ten (6 runs: 1 failure; then, typing
  at 60 ms per key instead of 25 ms, 12 runs: 1 failure; one full default-parallelism run passed and one failed). A different key goes missing each time and the Enter that follows survives, so it is not queue overflow at the end of the burst.
  Contention makes it worse, but the loss is in the guest's input path (the virtio-input event ring, or the keyboard IRQ racing the block-device IRQs during the copy) -- a kernel matter, outside this stage, which changes no kernel code.
  A failing run of just that check is worth one retry. **Decision (the user's): the input-path bug is deferred to the scheduling stage** (ROADMAP Stage 26, preemptive multitasking, where the console's input queue gets a kernel-side task of its own and stops depending on
  the interrupt landing while a program runs), not fixed here.

## Step 2 -- flag catch-up (overrides in `progs_r18`, reusing `progs_r16::{join, read_dir}`)
`mkdir -p -v`; `cp -r/-R -n -v`; `mv -n -v -f` (`-f` accepted: nothing prompts); `rm -v -d`; `ls -a -d -R -r -t -S -h` plus hiding dot-names. Fallout: `pipes.py`'s `ls /tmp` and any listing of a dot-name gain `-a`;
`core_utils`/`user_progs` derive `ls` output from the host directory, so they filter names starting `.`. Tests: group `flags`; docs rows say `From Stage 18:`.

**As built (Step 2).** As planned, with these specifics:
- Overrides `mkdir`, `cp`, `mv`, `rm`, `ls` in `progs_r18`, whose `lib.rs` now carries `join`, `read_dir`, `Entry` and `ReadDirError` (copied from `progs_r16`, not depended on: that crate pulls the time-zone database into the build) and the
  pure `human.rs` (`ls -h`'s sizes, host-tested: 5 tests, 222 in all).
- Behaviours taken from the host's coreutils rather than assumed: `cp -n` skips silently with status 0, `mv -n` says `mv: not replacing 'x'` and exits 1, `mkdir -p` reports a file in the way as `Not a directory` (middle) or `File exists` (last),
  `ls -R` prints `dir:` headers with a blank line between and `.:` for no operand, `rm -rv` lists each entry then its directory. `cp -r` and `rm -r` walk entries in name order (GNU's is unspecified) so output is deterministic.
- `ls`: hiding dot-names, listing a file operand as itself and sorting directory operands are **behaviour changes** (the old expectations were rewritten, not kept): `core_utils`' "ls file", `user_progs`' several-operand order, `pipes`' `ls /tmp` (now
  `ls -a`), and the five `--help` texts (moved into `flags.py`). `-a` shows no `.`/`..` (the kernel never returns them). `-l` is unchanged (flags and size; no time column).
- `cp -r tests/ct tests/ct` reports "into itself, 'tests/ct/ct'" because the destination is a directory (as GNU does); the same-file check only fires for a non-directory destination.
- Tests: group `flags` (88 checks; `ls -t` waits 2 x 2.4 s of real time between three files). `just test` = 1250 checks.

## Step 3 -- filters
Pure helpers in `progs_r18/src/{glob,cutlist,trset}.rs` (`no_std` + `alloc`, included into `hosttests` by `#[path]`): glob matcher (`* ? [a-z] [!..]`), `cut` list parser, `tr` set expander, `sort -n` key. Programs: `sort`, `uniq`, `cut`,
`tr`, `find` (children in name order for determinism), `fgrep` (`name:` prefix with several files; status 0/1/2). Whole input in the user heap; `ENOMEM` reported as `tail` does. Tests: group `filters` (`seq` for input).

**As built (Step 3).** As planned, with these specifics:
- Pure helpers in `progs_r18/src/`, all `no_std` + `alloc` and pulled into `r18_utils/hosttests` by `#[path]`: `glob.rs` (`* ? [a-z] [!x] \`, on characters, no special leading dot), `cutlist.rs` (parse and merge `N`, `N-M`, `N-`, `-M`), `trset.rs`
  (set expansion with ranges, escapes, octal and classes, and the compiled `Tr` for translate, delete and squeeze), `sortkey.rs` (bytewise and exact numeric comparison, with GNU's last-resort tiebreak), `textutil.rs` (`split_lines`, ASCII-folded
  `contains`). 36 new host tests: 258 in all. `progs_r18::read_all` reads a whole input with `try_reserve`, so a heap that cannot hold it is `Cannot allocate memory`, not a panic.
- `sort` puts all files' lines together; `-u` drops the last-resort tiebreak (equal keys are duplicates) and keeps the first of a set. `cut -c` counts UTF-8 characters (a line that is not UTF-8 is cut by bytes). `tr` sets are ASCII bytes and every other byte
  passes through. `find` visits entries in name order and starts a `.`-less argument list at `.`. `fgrep` is GNU's `fgrep` (fixed strings), statuses 0/1/2.
- A bug caught by the first run: `cut` used `cli::operands` for its files, but that second walk treats an option's *value* (`-d :`, `-f 2`) as an operand -- it is only for programs with no value-taking options (`head`/`tail` parse once for the same reason). `cut`
  now collects operands in its single pass.
- Tests: group `filters` (125 checks); rows for the six programs in `docs/progs.md` (`check-docs`: 35 programs). The harness cannot type non-ASCII, so multi-byte input comes from the existing `cjk.txt` fixture. `just test` = 1388 checks and 258 host tests, lint clean.

## Step 4 -- text-tool flags
`cat -n -E -T -s`; `head`/`tail` with several files (`==> f <==`, `-q`), `head -n -N`, `tail -n +N`; `wc -m`; `echo -e -E`. Tests: group `textflags`.

**As built (Step 4).** As planned, with these specifics:
- Overrides in `progs_r18`: `cat`, `echo`, `wc`, `head`, `tail`. The pure count parser is `countspec.rs` (a sign and a number; 2 host tests: 260 in all). `cat` with no flag is still the plain byte copy (binary-safe); with a flag it runs a small state machine (line number,
  "line has started", "last line was blank") that carries across files. `echo`'s options are the leading arguments made of a dash and only `n`/`e`/`E`, so `-ne` and `-en` work and `-x` is text. `wc` prints in GNU's order (lines, words, characters, bytes, longest line).
- `head` and `tail` gather their file operands in the single option pass (the `cut` lesson again: `cli::operands` would take an option's value for one). A file that cannot be opened gets an error and no header; the blank line between listings comes before the next
  header. `head -n -N` and `-c -N` hold the whole input in the heap (`read_all`); `tail -n +N` and `-c +N` are one streaming pass that reuses `emit_lines`/`emit_bytes`. `-` stays an ordinary file name in all of them (it was for `cat`; GNU treats it as stdin for `head`/`tail`).
- Behaviour changes, rewritten rather than kept: `tail -n -3` was an invalid count and is now the last three lines (GNU treats a minus as no sign), and the `--help` texts of `cat`, `echo`, `wc`, `head` and `tail` moved from `user_progs.py` into `textflags.py`.
- Tests: group `textflags` (79 checks); rows updated for the five programs in `docs/progs.md`. `just test` = 1468 checks and 260 host tests, lint and `check-docs` clean. (The first full run tripped the known `token_queue` flake, with the whole `echo intact` line missing this time; the rerun passed.)

## Step 5 -- docs, roadmap, sweep
`docs/progs.md`, `docs/tests.md`, `ROADMAP.md` Stage 18 "As built", `just check-docs`, `just lint`; regression: `r17_env` and the older stages' suites (only the new `progs_r18` crate is added to the shared tree), restoring any tracked build
artifacts with `git checkout`.
