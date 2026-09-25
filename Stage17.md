# Stage 17: shell variables, the environment, `$VAR`, `$?` -- `r17_env` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the Steps in the order they are built and committed. It is
updated as each Step lands (an "As built" note per Step, as `Stage12.md` does).

| Step | What | Status |
|---|---|---|
| 0 | Plain copy of `r16_brk` as `r17_env` | done |
| 1 | The variable model (exported flag, `export`, `unset`) and `/etc/environment`; `disk/home` -> `disk/root` | done |
| 2 | `envp` to programs; `userlib::env`; `env` and `printenv` (new tier `progs_r17`) | planned |
| 3 | Expansion: `$VAR`, `${VAR}`, `$?`, with field splitting | planned |
| 4 | Assignments: `NAME=value`, `NAME=value cmd` | planned |
| 5 | Retire the automatic `exit N` line | planned |
| 6 | `$HOME` for `cd`, `$TZ` for `date` and `stat` | planned |
| 7 | Docs, roadmap, regression sweep | planned |

## Context
Stage 17 (renumbered from old 16; see `ROADMAP.md`, "Before Capstone 1") gives the shell POSIX-shaped variables and gives programs a real
environment: `envp` on the stack next to `argv`, shell variables with an exported flag (`NAME=value`, `NAME=value cmd`, `export`, `unset`),
`$VAR`/`${VAR}` expansion and `$?`, retiring Stage 12's stand-in `exit N` line, `cd` with no operand going to `$HOME`, and `date`/`stat`
reading `$TZ` instead of the hard-coded `America/Toronto` (`LOCAL_ZONE` in `user/progs_r15/src/lib.rs`).

The work does not split cleanly into "copy / OS / user programs": the kernel's `envp` can only be observed through a user program,
expansion is pure shell logic independent of `envp`, assignments need both the lexer and the expander, and retiring `exit N` is ~54
mechanical test edits best reviewed on their own. So it is a copy plus six working steps, one commit each (the user commits; I stage after each step).

## Decisions (settled)
- **Variables are POSIX-shaped**: a frame holds shell variables, each with an `exported` flag. `NAME=value` on its own sets a shell variable (keeping its
  flag; new ones are not exported); `export NAME[=value]` sets the flag (and assigns); `unset NAME` removes; `$NAME` reads every variable; `envp` for a
  program is the *exported* ones. `NAME=value cmd` puts the value in that one command's environment only (an overlay, removed afterwards, also applied while a
  *builtin* runs so `HOME=/tmp cd` works). The right-hand side of an assignment goes through quoting and `$` expansion but is not field-split. A `./script` or
  `sh script` scope starts with only the exported variables (all marked exported), as a real child process would; `source` shares the frame. Names are
  `[A-Za-z_][A-Za-z0-9_]*`. Out of scope: `set -a`, `readonly`, `local`, arrays, `export -p`.
- **The initial environment comes from `/etc/environment`**, read once at boot -- the file Linux's `pam_env` reads: plain `NAME=VALUE` lines, `#` comment lines,
  blank lines; the value is everything after the first `=`, literal (no quotes, no expansion); a bad line is skipped with a note on the serial log; duplicate names:
  last wins; every variable read is exported. It lives at a fixed path, not under `$HOME`. **Missing file => empty environment** (a serial note; the shell still
  starts). A login script (`~/.profile`-style) is a possible later addition.
- **The home directory is `/root`** (what Linux gives a single root user): rename `disk/home` -> `disk/root` (it holds only `utf8-demo.txt`), add
  `disk/etc/environment`. General image: `HOME=/root` and `TZ=America/Toronto`. **Tests**: the harness writes `HOME=/` into each group's private image copy before
  boot (`mcopy -o`), and a module may override it with an `ENVIRONMENT` attribute (`clock` and `user_progs` use `HOME=/` + `TZ=America/Toronto`, keeping their
  Toronto-time expectations). Nothing in the kernel names `/home`; the only test edit is `core_utils.py`'s `ls -F` of `/` (`bin/ etc/ fonts/ root/ tests/ tmp/`).
- **Unquoted expansions are field-split on blanks** (POSIX); an unquoted empty expansion vanishes; `"$X"` stays one word (possibly empty). A redirect target that
  expands to other than one word is `ambiguous redirect`.
- `$?` = last pipeline's status: the program's exit status (a fault is 139); builtin success 0 / failure 1; `command not found` 127; found but not executable /
  cannot execute 126; syntax error 2. One global, not per frame.
- `envp` uses `x2` at `eret` (next to `argc`/`argv` in `x0`/`x1`); args and env share `ARG_MAX` (128 KiB). Unknown `$TZ` => UTC, like glibc. `cd` with `HOME`
  unset: `cd: HOME not set`.
- User side is opt-in: a **new** `userlib::entry_with_env!` macro (existing macros untouched, so r09-r16 binaries are unchanged).

## Step 0 -- plain copy `rust/r17_env`
`rsync` `r16_brk` -> `r17_env` (excluding `target`, `disk.img`, `*.elf`, `__pycache__`), rename `r16_brk` in `Cargo.toml`/`Cargo.lock`, `justfile` `BIN`,
`test/check_docs.py`, `test/run_tests.py` (temp-dir prefix), `test/README.md`, `disk/tests/notes.txt`, `disk/fonts/NOTICE`, `hosttests/src/lib.rs`.
Tier list unchanged. Verify `just test` = 643 checks, nothing else changed.

## Step 1 -- the variable model + `/etc/environment`
- `src/exec/frame_stack.rs`: `ShellFrame` gains `vars: Vec<Var{name, value, exported}>` (insertion order) with `get/set/export/unset/exported()`, name validation;
  `with_scope` gets a variant for a *child process* (`with_child_scope`: only the exported variables, all exported) beside the existing copy used by `source`-style sharing
  (which pushes nothing); `with_stdio` leaves vars alone. Host tests (hosttests already pulls this file): set/replace/unset, the exported flag, child vs shared scoping.
- `src/shell/builtins.rs`: `export` (`NAME`, `NAME=VALUE`), `unset`; bash-worded errors (`export: 'a-b': not a valid identifier`). (Plain `NAME=value` arrives in step 4.)
- New `src/shell/environment.rs`: pure `parse(text) -> (Vec<(String,String)>, Vec<Problem{line,why}>)` (host-tested: comments, blanks, no `=`, empty name, invalid name,
  `=` inside the value, CRLF, duplicates) plus `load()` reading `/etc/environment` via `files::lookup` + `read_file_checked` (`src/fs/mod.rs`), called from `main.rs`
  just before `shell::run()` (after `FRAMES`/`VOL` are set), notes via `uart_write`; variables enter the bottom frame exported.
- Image and tests: rename `disk/home` -> `disk/root`, add `disk/etc/environment`; `test/harness.py` gains `mcopy_in`; `test/run_tests.py::run_group` writes the environment
  file into the image copy before `Session(...)`; update `core_utils.py`'s root `ls -F`. QEMU tests: boot-log notes for a bad file (a group with an `ENVIRONMENT` containing a bad
  line, and a missing-file variant), `export`/`unset` errors. (Values are not yet observable from EL0 -- step 2.)

**As built (Step 1).** As planned, with these specifics:
- `frame_stack.rs`: `Var { name, value, exported }`, `ShellFrame.vars`, `set_var`/`export_var`/`unset_var`/`var`/`is_exported`/`exported()` and `is_valid_name`; `with_scope`
  now pushes a *child* frame (`push_child`: cwd and streams, exported variables only, all exported); the old `push_copy` is gone. 12 new host tests.
- `shell/environment.rs` (pure): a line is `NAME=VALUE`, blanks around the name trimmed, the value literal, `#` and blank lines skipped, CRLF tolerated, a repeated name
  takes its last value in its first place; problems carry a line number and a reason. `hosttests` gained an `exec` alias module so a file that reaches across directories
  compiles unchanged in both trees. `shell::load_environment` (in `shell/mod.rs`) runs in `main.rs` after the volume mounts and *before the first prompt*, with the local
  volume (the statics come later), so its notes come first on the serial log; `fs::read_path` reads a file by absolute path from a volume.
- `export` and `unset` builtins: bash's wording; `export` alone and `-p`, and `unset -v`, are refused (a builtin has no stdout to list on).
- Image: `disk/home` -> `disk/root`; `disk/etc/environment` holds `HOME=/root` and `TZ=America/Toronto`. Tests write `HOME=/` (or a module's `ENVIRONMENT`) into each
  group's copy of the image (`harness.set_environment`, `mcopy -o`; `None` removes the file). New modules `environment`, `env_bad`, `env_missing`.
- `QEMU_TEST_WORKERS=N` overrides the runner's parallelism. Three QEMU processes left running from earlier sessions were using three cores, and with 19 sessions in parallel the
  timing-sensitive `token_queue` check (typing during a large copy) started failing every time; with 12 workers it is stable.

## Step 2 -- `envp` to programs, `env` and `printenv`
- `src/exec/argplan.rs`: generalize `plan` to args + env: strings, then `argv[]`+NULL, then `envp[]`+NULL, 16-byte aligned; `ArgsPlan` gains `envp`. Host tests (existing
  table tests + env cases + the shared `ARG_MAX` limit).
- `src/exec/process.rs`: `push_cstr_array` writes both; `prepare(elf, args, env)`, `PreparedProgram.envp`, `in("x2") program.envp` in `run`'s inline asm.
  `src/shell/launch.rs` passes the exported variables of the top frame as `NAME=VALUE` strings; `E2BIG` covers both.
- `user/userlib`: new `env` module -- `entry_with_env!` (stores `x2` in a static), `env::var(name) -> Option<&'static str>`, `env::vars()`. `abi` unchanged. A program built for an
  older kernel never calls it, so a garbage `x2` there is harmless.
- New tier `user/progs_r17`: `env` (prints `NAME=VALUE` lines) and `printenv [NAME]` (value, exit 1 if unset); register the tier in `r17_env/justfile` (`for tier in ...`, `lint`), rows
  in `docs/progs.md`.
- QEMU tests (`test/cases/env.py`): inheritance (`export FOO=bar` then `printenv FOO`), replace/unset, a script's scope vs `source`, an *unexported* variable not reaching a child (once
  step 4 can make one), the environment file's variables visible (`printenv HOME` = `/` under the test file; an `ENVIRONMENT` override group with `TZ`), an environment too big for
  `ARG_MAX` refused (`E2BIG`), `envp` layout probe (`probe env`: NULL-terminated, aligned, count).

## Step 3 -- expansion: `$VAR`, `${VAR}`, `$?`
- `src/shell/lexer.rs` (`peg`): stop treating `$` as ordinary. A word becomes a list of parts (`Text`, `Var(name)`, `Status`) each carrying whether it was quoted; `\$` (in double quotes) and
  single quotes stay literal, as the lexer's own doc comment already promises. Update `Token::Word`, the module doc, and `docs/shell.ebnf`.
- `src/shell/syntax.rs`: `Segment.argv` holds unexpanded words; redirect targets likewise.
- New pure `src/shell/expand.rs`: `expand(words, &lookup, status) -> Result<Vec<String>>` with the field splitting above, and `expand_one` for redirect targets and (later) assignment
  values (`ambiguous redirect`). Host-tested (quoting matrix, `${X}` vs `$X`, `$?`, empty vanishes, split on runs of blanks, a `$` not followed by a name stays literal).
- `src/shell/mod.rs`: expand each segment just before it runs. `launch` returns a status for every outcome (127/126 mapping above) instead of `Option<i32>`; `run_line_inner` stores the
  last pipeline's status in `shell_state`. **The `exit N` line still prints in this step.**
- QEMU tests: `echo $FOO` (via `export`), `"${FOO}bar"`, `'$FOO'` literal, `echo $?` after success/failure/fault/not-found/not-executable/syntax error, field splitting into two args
  (`probe args`), expansion in a redirect target.

## Step 4 -- assignments: `NAME=value`, `NAME=value cmd`
- Lexer/syntax: a leading word of the form `NAME=...` with an unquoted valid name (before the first command word) is an assignment; `Segment` gains `assignments: Vec<(String, Word)>`.
  Grammar/`docs/shell.ebnf` updated (an `assignment` production ahead of the command word).
- Execution (`src/shell/mod.rs`): no command word => set the shell variables (value expanded, not field-split; existing flag kept); with a command word => an environment overlay applied
  for that command only (programs get it in `envp`; a builtin sees it while it runs), like `with_stdio`'s save/restore; an assignment value is expanded before *any* assignment of the
  same line takes effect (`FOO=1 echo $FOO` sees the old `$FOO`). A pipeline stage's assignments belong to that stage.
- Tests: `FOO=bar` then `echo $FOO`, unexported not visible to `printenv`, `export FOO` then visible, `FOO=x printenv FOO` (and gone afterwards), `HOME=/tests cd` (with `pwd`), quoting in
  values, an assignment in a `./script` not leaking but in a `source`d one persisting, `FOO=` (empty) vs unset.

## Step 5 -- retire the automatic `exit N` line
- `src/shell/mod.rs`: remove the `report` parameter and the print in `run_segment_with`/`run_pipeline`; the status only feeds `$?`.
- Tests (~54 expectations across `core_utils`, `user_progs`, `launch`, `cwd`, `redirection`, `pipes`, `power`, `stack`): drop the trailing `exit N` line; `stack.py`'s `faults()` helper checks the
  segfault message alone. Add a harness helper `s.status(cmd)` (runs `cmd`, then `echo $?`) for the handful of checks where the status matters (fault = 139, `false` = 1, not found = 127).
- Docs: `shell.md` and `progs.md`'s "Errors" convention (statuses are visible through `$?`, nothing is printed); `Stage12.md` left as history.

## Step 6 -- `$HOME` and `$TZ`
- `src/shell/builtins.rs`: `cd` with no operand goes to `$HOME` (`cd: HOME not set` otherwise); update its doc and `shell.md`'s builtin table.
- `user/progs_r17`: `date` and `stat` copied from `progs_r16`, `LOCAL_ZONE` replaced by `env::var("TZ")` parsed with `chrono_tz::Tz::from_str` (**verify `FromStr` is available with
  `default-features = false`; if not, enable the feature that provides it**), UTC when unset or unknown; `-u` still forces UTC. `progs_r15`/`progs_r16` versions stay for their kernels.
- Tests: `clock.py`/`user_progs.py` use `ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"`; add `TZ=America/Vancouver date ...` (prefix form), `export TZ=...`, `unset TZ` => UTC, a bad name => UTC;
  `cd` no-operand with `HOME` set/unset/reassigned.

## Step 7 -- docs, roadmap, sweep
`docs/shell.md` (variables, assignment, `export`, expansion, `$?`, the retired line), `docs/progs.md` (`env`, `printenv`, `date`/`stat` and `$TZ`), `docs/launching_programs.md` (`envp` layout
and `x2`), `docs/filesystem.md` (`/etc/environment`, `/root`), `docs/tests.md` (environment-file injection, `ENVIRONMENT`), repoint `docs/*` from `r16_brk` to `r17_env`, `ROADMAP.md` Stage 17
"As built" (and the `/etc/environment` + `/root` decisions), `just check-docs`, `just lint`. Regression: `r16_brk`, `r15_large_binaries`, ... `r11_busybox` (userlib and `progs` are shared:
only additive changes allowed).

## Critical files
`rust/r17_env/src/{exec/frame_stack.rs, exec/argplan.rs, exec/process.rs, shell/{lexer,syntax,builtins,launch,mod}.rs, shell/{environment,expand}.rs (new), main.rs}`,
`rust/user/userlib/src/{process.rs or new env.rs, lib.rs}`, `rust/user/progs_r17/` (new tier), `rust/r17_env/test/{harness.py, run_tests.py, cases/*}`,
`rust/r17_env/disk/{etc/environment (new), root/ (renamed from home/)}`, `rust/docs/shell.ebnf`. Reused as-is: `files::lookup`/`read_file_checked` (loading the file), `argplan`/`push_cstr_array`
(extended, not replaced), `FrameStack::with_scope`/`with_stdio` (the pattern for child scopes and the command overlay), `progs_r12::cli` (new programs' argument parsing), the `heap` feature.

## Verification (every step)
`just test-host` (frame vars, argplan, environment-file parser, expander), `just test` in `r17_env` (QEMU groups), `just lint`, `just check-docs`; after any step that touches
`userlib`/`abi`/`progs`: run `r16_brk` ... `r11_busybox`. Manual check on the display at the end: `cat /etc/environment`, `env`, `FOO=1`, `export FOO`, `echo $TZ`, `TZ=UTC date`, `cd`.

## Risks / open items to confirm while building
- The `peg` lexer change (steps 3-4) is the riskiest edit (existing tests pin `$` as literal); write the expansion/assignment tests first.
- `chrono-tz` `FromStr` without default features (step 6); if it needs `alloc`, the heap is already there for these programs.
- Unset/garbage `x2` on older kernels only matters to programs that call `env::var` -- none built for them.
- Assignment recognition must not misfire on words like `a=b` after the command word (only leading words are assignments) or on quoted names (`"A"=b`).
- Out of scope: `set -a`, `readonly`, `local`, arrays, `$@`/`$1` positional parameters, tilde and globbing, `export -p`, a login script.
