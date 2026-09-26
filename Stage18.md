# Stage 18: more utilities -- `r18_utils` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the steps in the order they are built and committed. It is updated as each step lands (an "As built" note per step, as
`Stage17.md` does). Programs only: **no kernel or shell change**, a new tier `user/progs_r18` on the scheme every stage since 12 uses (the highest tier wins a name).

| Step | What | Status |
|---|---|---|
| R | Insert the stage in the roadmap (Stage 18; the storage stage becomes 19, the editor 20, job control 21-26) | done |
| 0 | Plain copy of `r17_env` as `r18_utils` | done |
| 1 | The tier `progs_r18`; `rmdir`, `touch`, `seq`, `cmp` | planned |
| 2 | Flag catch-up: `mkdir -p -v`, `cp -r -n -v`, `mv -n -v -f`, `rm -v -d`, `ls -a -d -R -r -t -S -h` (dotfiles hidden by default) | planned |
| 3 | Filters: `sort`, `uniq`, `cut`, `tr`, `find`, `fgrep`; pure helpers host-tested | planned |
| 4 | Text-tool flags: `cat -n -E -T -s`, `head`/`tail` several files and `-n -N`/`-n +N`, `wc -m`, `echo -e -E` | planned |
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

## Step 2 -- flag catch-up (overrides in `progs_r18`, reusing `progs_r16::{join, read_dir}`)
`mkdir -p -v`; `cp -r/-R -n -v`; `mv -n -v -f` (`-f` accepted: nothing prompts); `rm -v -d`; `ls -a -d -R -r -t -S -h` plus hiding dot-names. Fallout: `pipes.py`'s `ls /tmp` and any listing of a dot-name gain `-a`;
`core_utils`/`user_progs` derive `ls` output from the host directory, so they filter names starting `.`. Tests: group `flags`; docs rows say `From Stage 18:`.

## Step 3 -- filters
Pure helpers in `progs_r18/src/{glob,cutlist,trset}.rs` (`no_std` + `alloc`, included into `hosttests` by `#[path]`): glob matcher (`* ? [a-z] [!..]`), `cut` list parser, `tr` set expander, `sort -n` key. Programs: `sort`, `uniq`, `cut`,
`tr`, `find` (children in name order for determinism), `fgrep` (`name:` prefix with several files; status 0/1/2). Whole input in the user heap; `ENOMEM` reported as `tail` does. Tests: group `filters` (`seq` for input).

## Step 4 -- text-tool flags
`cat -n -E -T -s`; `head`/`tail` with several files (`==> f <==`, `-q`), `head -n -N`, `tail -n +N`; `wc -m`; `echo -e -E`. Tests: group `textflags`.

## Step 5 -- docs, roadmap, sweep
`docs/progs.md`, `docs/tests.md`, `ROADMAP.md` Stage 18 "As built", `just check-docs`, `just lint`; regression: `r17_env` and the older stages' suites (only the new `progs_r18` crate is added to the shared tree), restoring any tracked build
artifacts with `git checkout`.
