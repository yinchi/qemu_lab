# Stage 17: shell variables, the environment, `$VAR`, `$?` -- `r17_env` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the Steps in the order they are built and committed. It is
updated as each Step lands (an "As built" note per Step, as `Stage12.md` does).

| Step | What | Status |
|---|---|---|
| 0 | Plain copy of `r16_brk` as `r17_env` | done |
| 1 | The variable model (exported flag, `export`, `unset`) and `/etc/environment`; `disk/home` -> `disk/root` | done |
| 2 | `envp` to programs; `userlib::env`; `env` and `printenv` (new tier `progs_r17`) | done |
| 3 | Expansion: `$VAR`, `${VAR}`, `$?`, with field splitting | done |
| 4 | Assignments: `NAME=value`, `NAME=value cmd` | done |
| 5 | Retire the automatic `exit N` line | done |
| 6 | Pipeline stages run in a subshell (a discarded copy of the shell's frame) | done |
| 7 | `$HOME` for `cd` and for where the init shell starts, `$TZ` for `date` and `stat` | done |
| 8 | `$PATH`: the directories a bare command name is looked up in | planned |
| 9 | `$PS1`: a limited prompt string (working directory) | planned |
| 10 | `~/.profile`: a per-user start-up script, run by the init shell | planned |
| 11 | Docs, roadmap, regression sweep | planned |

## Context
Stage 17 (renumbered from old 16; see `ROADMAP.md`, "Before Capstone 1") gives the shell POSIX-shaped variables and gives programs a real
environment: `envp` on the stack next to `argv`, shell variables with an exported flag (`NAME=value`, `NAME=value cmd`, `export`, `unset`),
`$VAR`/`${VAR}` expansion and `$?`, retiring Stage 12's stand-in `exit N` line, `cd` with no operand going to `$HOME`, and `date`/`stat`
reading `$TZ` instead of the hard-coded `America/Toronto` (`LOCAL_ZONE` in `user/progs_r15/src/lib.rs`).

The work does not split cleanly into "copy / OS / user programs": the kernel's `envp` can only be observed through a user program,
expansion is pure shell logic independent of `envp`, assignments need both the lexer and the expander, and retiring `exit N` is ~54
mechanical test edits best reviewed on their own. So it is a copy plus ten working steps, one commit each (the user commits; I stage after each step).

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
  compiles unchanged in both trees. `shell::load_environment` (in `shell/mod.rs`) runs *before the first prompt*, so its notes come first on the serial log (Step 7 moved it from `main.rs` into the shell's own start-up, below);
  `fs::read_path` reads a file by absolute path from a volume.
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

**As built (Step 2).** As planned, with these specifics:
- `argplan::plan(sp, floor, arg_lens, env_lens)` returns `ArgsPlan { args, env, argv, envp }`; one 16-byte-aligned pointer block holds `argv[]`, its NULL, `envp[]`, its NULL, so `envp`
  is `argv + (argc + 1)` slots (Linux's layout) and `argv` is the initial `sp`. `process::push_strings` replaces `push_cstr_array`; `ARG_MAX` counts both, as Linux's does.
- `userlib::env` is a public module (`env::var`, `env::vars`), not flat-re-exported; a program started with `entry!`/`entry_with_args!` sees an empty environment. `progs_r17`: `env`
  (no options or operands; GNU's command-running form is the shell's job in Step 4) and `printenv [NAME]...` (several names allowed, as GNU; status 1 if any is unset).
- Tests: new `env` group (its own `ENVIRONMENT`, with a space, quotes, `$` and an empty value in the file), `probe env`, and an `E2BIG` case using `tests/bigenv.sh` (generated by
  `mkfixtures.py`: 150000 bytes of exports; gitignored like `biglines.txt`), plus the `env` checks added to `environment`, `env_bad` and `env_missing`, which close Step 1's gap of
  never reading the loaded variables back. The unexported-variable case waits for Step 4.

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

**As built (Step 3).** As planned, with these specifics:
- Lexer: `Token::Word(Word)`, `Word { parts: Vec<Part> }`, `Part::{Lit, Var{name, quoted}, Status{quoted}}`. Text is a `Lit` whatever quoting it had (only an expansion's result cares about quotes),
  so the old `quoted` flag is gone; `""` stays a `Lit("")`, which keeps an empty word from vanishing. A `$` that names nothing (`$1`, `$$`, `$ `, `$` at the end) is text. `${` not followed by `NAME}`
  is `LexError::BadSubstitution` (`syntax error: bad substitution`, status 2). The sentinel needs `&braced()` ahead of `braced()` so peg's furthest-failure rule does not hide it.
- `Segment.argv` is `Vec<Word>`, redirect paths are `Word`; `>&`'s target must be a literal digit (`>&$x` is `BadDupTarget`).
- `shell/expand.rs`: `expand(words, &Values{lookup, status})`, `expand_word`, `expand_target` (exactly one field or `Ambiguous`, worded `<word>: ambiguous redirect`, status 1). The no-split
  form for assignment values waits for Step 4, its only user.
- Statuses: `shell_state::last_status()`/`set_last_status()` (one atomic for the whole shell). `run_line_inner` returns `Option<i32>` (`None` for a blank line or comment, which leaves `$?` alone) and
  records it; a syntax error is 2. `launch` returns `Launched { status, ran }` -- `ran` only so the `exit N` line still prints exactly where it did (Step 5 removes the field): 127 not found
  (including a path that does not exist), 126 for anything found but not runnable, a script's own last-line status for `./script`. `builtins::run` returns `Result<i32, String>` (`source`/`sh`: the
  script's status; other builtins 0, or 1 on error); `run_script_content` returns `Result<i32, String>`. A redirect that fails is status 1.
- Words are expanded in `run_segment_with`, before the redirects, so a stage sees `$?` as of the previous *line*.
- Tests: new `expansion` group (plus the `syntax` group's `$` cases updated: `$x` is no longer text). 806 checks in all.

## Step 4 -- assignments: `NAME=value`, `NAME=value cmd`
- Lexer/syntax: a leading word of the form `NAME=...` with an unquoted valid name (before the first command word) is an assignment; `Segment` gains `assignments: Vec<(String, Word)>`.
  Grammar/`docs/shell.ebnf` updated (an `assignment` production ahead of the command word).
- Execution (`src/shell/mod.rs`): no command word => set the shell variables (value expanded, not field-split; existing flag kept); with a command word => an environment overlay applied
  for that command only (programs get it in `envp`; a builtin sees it while it runs), like `with_stdio`'s save/restore; an assignment value is expanded before *any* assignment of the
  same line takes effect (`FOO=1 echo $FOO` sees the old `$FOO`). A pipeline stage's assignments belong to that stage.
- Tests: `FOO=bar` then `echo $FOO`, unexported not visible to `printenv`, `export FOO` then visible, `FOO=x printenv FOO` (and gone afterwards), `HOME=/tests cd` (with `pwd`), quoting in
  values, an assignment in a `./script` not leaking but in a `source`d one persisting, `FOO=` (empty) vs unset.

**As built (Step 4).** As planned, with these specifics:
- The lexer decides what *could* be an assignment (only it knows what was quoted): a word starting with an unquoted valid `NAME=` carries `Word.assign = Some(len)`; `Word::assignment()` splits it into the name and a value
  word. `syntax.rs` makes it one only before the first non-assignment word (`Segment.assignments`); `A=1` alone is a valid command. `Word::from_parts` is now the public constructor.
- `expand::expand_assignment` joins the parts with no splitting. `run_segment_with` expands the command's words first, then applies the redirects, then the assignments (`apply_assignments`), runs, and
  `restore_assignments`. Overlay = `export_var(name, value)` after `ShellFrame::saved_var`; `restore_var` puts back value and flag (and unsets a variable that did not exist), last first. A command whose words all expand to nothing
  makes its assignments permanent, as in bash.
- `export`'s operands of the form `NAME=value` are expanded as assignments (`expand::expand_command`, bash's rule for declaration commands): `export A=$X` does not split `X`. Only `export` is one here.
- Deliberate difference from bash: every stage of a pipeline shares the frame, so `A=1 | cat` sets `A` in the shell.
- Tests: new `assignment` group and `tests/assign.sh`; 874 checks.

## Step 5 -- retire the automatic `exit N` line
- `src/shell/mod.rs`: remove the `report` parameter and the print in `run_segment_with`/`run_pipeline`; the status only feeds `$?`.
- Tests (~54 expectations across `core_utils`, `user_progs`, `launch`, `cwd`, `redirection`, `pipes`, `power`, `stack`): drop the trailing `exit N` line; `stack.py`'s `faults()` helper checks the
  segfault message alone. Add a harness helper `s.status(cmd)` (runs `cmd`, then `echo $?`) for the handful of checks where the status matters (fault = 139, `false` = 1, not found = 127).
- Docs: `shell.md` and `progs.md`'s "Errors" convention (statuses are visible through `$?`, nothing is printed); `Stage12.md` left as history.

**As built (Step 5).** As planned, with these specifics:
- `launch` and the segment runners return a plain `i32` again: `Launched { status, ran }` and `run_segment_with`'s `report` parameter are gone. Nothing prints a status.
- Tests: 86 expectations ended in an `exit N` line. A script rewrote them from the AST (`check(name, s.run(cmd), want)` with a trailing `exit N\n` became `check(name, s.run_status(cmd), (want, N))`), so
  each still verifies the status it used to show; `FAULT` constants became a plain `"Segmentation fault" in out`. New harness helpers `Session.run_status(cmd) -> (transcript, status)` and `Session.status(cmd)`.
  A check that goes on to read `$?` itself must use plain `run` (a `run_status` leaves `$?` as its own `echo` set it). `tests/assign.sh` now prints `status $?` after each `printenv`, since a
  silent `printenv` no longer shows its failure. 881 checks in all (the total rose because most of those converted checks now cost a second command).
- Docs: `shell.md` (the `exit N` section replaced by one paragraph), `progs.md`'s Errors convention, `syscalls.md`, `tests.md`. `Stage12.md` and ROADMAP's Stage 12 text stay as history.

## Step 6 -- pipeline stages run in a subshell
Steps 3-5 left one known difference from a POSIX shell: every stage of a pipeline shares the shell's one frame, so `echo $FOO | unset FOO` removes `FOO` from the shell (bash's stage is a
subshell and leaves it), and `A=1 | cat`, `cd d | cat` and `export X=1 | cat` all leak. This step closes it; it is the same isolation `./script` has, but a subshell is a *fork*, not an exec, so it
inherits everything.
- `src/exec/frame_stack.rs`: `push_subshell` / `with_subshell` -- run against a full copy of the top frame (cwd, streams, **every** variable with its exported flag), then discard it. This is not
  `with_scope` (a `./script` child inherits only the exported variables, all marked exported): `X=1` then `echo $X | cat` must still see the unexported `X`. Host tests: the copy is
  complete, nothing done inside (`cd`, `set_var`, `unset_var`, `export_var`, stream rebinding) survives, and it nests inside `with_scope` and `with_stdio`.
- `src/shell/mod.rs` `run_pipeline`: each stage's `run_segment_with` runs inside `with_subshell`, the last stage included (bash's default, no `lastpipe`). A one-stage line is not a pipeline and still
  runs in the shell itself (`cd`, `export`, `A=1` alone must stick). `$?` is still the last stage's status, recorded after the pipeline (a global, so unaffected by the discarded frames).
- Tests (`pipes.py`, a few lines each): `FOO=123` then `echo $FOO | unset FOO` leaves `FOO` (and the stage still printed 123); `cd /tests | cat` leaves the directory; `export X=1 | cat`
  and `A=1 | cat` leave nothing; `source` in a stage is confined to it; an unexported variable is still readable in a later stage (`X=1` then `echo $X | cat`); a redirect and `$?` are as before;
  a single command still changes the shell (`cd`, `export`, `A=1`). Whatever `pipes.py`'s existing checks assumed about leaking (if any) is corrected, not preserved.
- Docs: `shell.md` (Pipelines: each stage is a subshell; the `A=1 | cat` note in "Variables and assignments" goes; the frame-stack section names the third operation) and the "As built" note.
  The roadmap's Stage 24 keeps its own line: it replaces the temp-file *pipes* with streaming ones between real processes; the isolation of stages is already here.

**As built (Step 6).** As planned, with these specifics:
- `FrameStack::push_subshell` / `with_subshell` (a plain clone of the top frame; three host tests: the copy is complete, nothing inside survives, and it nests with `with_scope`/`with_stdio`).
  `run_pipeline` wraps each stage's `run_segment_with` in it; the temp-file opens stay outside, in the shell's frame.
- Tests: a subshell section in `pipes.py` (`echo $FOO | unset FOO` leaves `FOO` and the stage still read the unexported `FOO`; unset first/last stage; `cd`, `export`, `A=1`, `source` confined; the lone forms,
  a lone command with a redirect included, still change the shell; an earlier stage's assignment does not reach a later stage). 911 checks in all.
- Docs: `shell.md` (Pipelines, Variables and assignments' scope note, the frame-stack section now lists three operations). No roadmap change yet; Step 11's Stage 17 "As built" records it, and Stage 24 is
  unchanged (it replaces the temp-file pipes, not the isolation).

## Step 7 -- `$HOME` and `$TZ`
- **The init shell starts in `$HOME`** (added after Step 6): no login or user system, so `shell::enter_home` does the `chdir` itself, once, before the first prompt; no or empty `HOME` => `/`, a `HOME` that is no directory => a serial note and `/`.
- `src/shell/builtins.rs`: `cd` with no operand goes to `$HOME` (`cd: HOME not set` otherwise); update its doc and `shell.md`'s builtin table.
- `user/progs_r17`: `date` and `stat` copied from `progs_r16`, `LOCAL_ZONE` replaced by `env::var("TZ")` parsed with `chrono_tz::Tz::from_str` (**verify `FromStr` is available with
  `default-features = false`; if not, enable the feature that provides it**), UTC when unset or unknown; `-u` still forces UTC. `progs_r15`/`progs_r16` versions stay for their kernels.
- Tests: `clock.py`/`user_progs.py` use `ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"`; add `TZ=America/Vancouver date ...` (prefix form), `export TZ=...`, `unset TZ` => UTC, a bad name => UTC;
  `cd` no-operand with `HOME` set/unset/reassigned.

**As built (Step 7).** As planned, with these specifics:
- `cd` with no operand reads `$HOME` when it runs (an overlay counts: `HOME=/fonts cd`); unset or empty is `cd: HOME not set`, status 1, directory unchanged.
- **The shell starts itself.** `kernel_main` now only brings the machine up (allocator, MMU, GIC, disk, filesystem, GPU/console, keyboard), puts every static in place -- `FRAMES` (an empty frame), `VOL` and
  the rest, none read by an interrupt handler -- enables the keyboard interrupt, and calls `shell::run()`. `run()` begins with `start_up()`: `load_environment`, `enter_home`, then the first prompt and flush,
  and only then the read-eval loop. (An earlier cut had `enter_home` in `main.rs` after the prompt was drawn, which put its serial note after the `> `; the harness waits for a log that ends with the prompt, so
  everything the shell says at start-up must come before it. Loading the environment and entering `$HOME` are also things a shell does for itself, not steps to interleave with hardware set-up.)
  `load_environment` and `enter_home` are private to `shell/mod.rs` and read the statics; the serial log now reads `Keyboard found ...`, then the `Environment:` notes, then `> `.
- `progs_r17::local_zone()` = `$TZ` parsed with `Tz::from_str` -- **no chrono-tz feature needed** (the plan's risk) -- with a leading `:` stripped, UTC for unset, empty or unknown; `date` and `stat` in `progs_r17`
  use it via `entry_with_env!`; `progs_r15`/`progs_r16` keep `LOCAL_ZONE` for their kernels. `progs_r17` now depends on `chrono`, `chrono-tz`, `getargs` and `userlib`'s `heap` feature.
- Tests: `clock` and `user_progs` get `ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"`; `clock` gains the `$TZ` section (prefix, `export`, plain assignment keeps the flag, `unset`, empty, unknown, case, POSIX rule string,
  `EST5EDT`, `-u`, `stat` in four zones); new `home` and `home_bad` groups, and `env_missing` checks `pwd` and `cd`. 967 checks in all.

## Step 8 -- `$PATH`
Today `launch.rs::find_program` looks a bare name up in `/bin` only (as `name`, then `name.exe`). This makes the directory list a variable.
- `src/shell/launch.rs`: a bare command name (no `/`) is looked up in each directory of `$PATH`, in order, trying `name` then `name.exe` in each (the existing rule, per directory); the first regular file found wins. A word
  containing `/` is still a path and bypasses `PATH`. **`PATH` unset means `/bin`**, so an environment-less boot and every existing test behave as before; **set but empty means no directory** (`command not found`).
  Empty *entries* (`a::b`, a leading or trailing `:`) are skipped -- POSIX would make them the working directory; skipping is safer and is documented. A relative entry is resolved against the working directory,
  as in bash. A file found but without the exec bit is still `Permission denied` (the first match decides, as today), a directory of that name is skipped.
- The splitting and the candidate list (`["/bin/x", "/bin/x.exe", ...]`) are a pure function in a new `src/shell/path_search.rs` (`candidates(path: Option<&str>, name: &str) -> Vec<String>`), host-tested: unset, empty,
  several entries, empty entries, relative entries, trailing slashes (`/bin/` gives `/bin/x`), a name that already ends in `.exe`.
- `disk/etc/environment` gains `PATH=/bin`. Test groups leave it unset unless a module sets it (`ENVIRONMENT`).
- Tests (`path.py`, its own group): a second directory of programs (`cp /bin/echo.exe /tests/bin/...`-style setup) found via `PATH=/bin:/tests/bin`, precedence when both directories hold one, `PATH` unset => `/bin`,
  `PATH=` empty => not found, an empty entry skipped, a relative entry, a prefix (`PATH=/tests/bin cmd`) for one command, `PATH` exported to a program (`printenv PATH`), the `.exe` fallback in the second directory, a `/` in the
  name bypasses it, and `$?` = 127 / 126 as before.
- Docs: `shell.md` "Program lookup" (currently says "no `PATH`-style search") and the builtin/limits text; `progs.md` intro sentence about `/bin`.

## Step 9 -- `$PS1` (limited)
The shell starts in `$HOME` and the prompt is a fixed `> `, so `pwd` is the only way to see where you are. A prompt variable fixes that, in a small form.
- `src/shell/prompt.rs` (pure, host-tested): `render(ps1: Option<&str>, cwd: &str) -> String`. Literal text with four backslash escapes: `\w` the working directory, `\W` its last component (`/` for the root), `\$` a `#` (there is
  only root), `\\` a backslash. Any other backslash sequence stays as typed. **No `$` expansion and no `\n`** (bash goes further; a newline would break the line editor's one-prompt-row assumption). Control
  characters are dropped so a prompt cannot move the cursor. Unset or empty `PS1` gives the default `> `.
- `src/keyboard/line_discipline.rs` (and `line.rs` if it names the prefix): the prompt prefix becomes an owned `String` instead of `&'static str`; its width (for wide characters too) feeds `rows_needed` and the redraw
  exactly as the constant did. `shell::start_prompt` renders `PS1` from the shell's own frame each time a prompt is drawn, so an assignment takes effect on the next prompt; the serial log gets the same text.
  `PS1` is an ordinary variable: not exported by default; `/etc/environment` (or `export`) may set it. Whether the general image's `/etc/environment` sets `PS1=\w> ` is a
  decision for when this lands (the default `> ` leaves every transcript unchanged; the harness's "log ends with `> `" contract holds for any `PS1` ending in `> `).
- Tests (`prompt.py`, its own group, plus host tests): default, `PS1='\w> '` after `cd` (the prompt shows the new directory at the next line), `\W`, `\$`, `\\`, unknown escapes literal, empty and unset => `> `, a prompt with
  wide characters, a prompt long enough to wrap the row with editing (Home/End/backspace) still redrawing correctly, history recall unaffected, and `PS1` set in `/etc/environment` in effect at the first prompt.
- Docs: `shell.md` (Starting up, the prompt paragraph in the line-editing text), `console.md` if it states the prompt is constant.

## Step 10 -- `~/.profile`
`/etc/environment` is data: `NAME=VALUE`, no expansion, so it cannot say `PATH=$PATH:/root/bin`. A per-user start-up script can, because the shell now has assignments, `$VAR` and `source`. Decision: a script (a plain
second environment file adds nothing over `export A=b` lines in one), following Linux, where `/etc/environment` stays a system-wide data file and the shell's profile is a script layered on top.
- `src/shell/mod.rs` `start_up`: after `enter_home` (so the shell is in `$HOME`, as after a login) and before the first prompt, run `$HOME/.profile` if it exists -- `run_script_content(content, false, 0)`, **unscoped**, so its
  assignments, `export`s, `cd`s and `PS1`/`PATH` changes are the shell's own, as `source` would. No `HOME`, or no such file: nothing happens, silently (the environment file *notes* a missing file; a profile is optional).
  A file that is not text, is too large (64 KiB, as `/etc/environment`) or is a directory: one serial note and it is skipped. A failing line reports and the script goes on, as any script does; that output, and any a command in it
  prints, reaches the console before the first prompt.
- Straight-line only: the shell has no control flow. `$?` after start-up is the profile's last command's, which is reset to 0 before the first prompt so a broken profile does not show in `echo $?` (decide when implementing).
- `disk/root/.profile` ships a default: a comment header and, if Step 8/9 want them there, `PATH`/`PS1` lines (kept commented so the general image's behaviour is the environment file's). Test groups have no profile unless a
  module supplies one: `PROFILE = "..."` (like `ENVIRONMENT`), written to `$HOME/.profile` in the group's image copy by the harness (`set_profile`), `None` removes it.
- Tests (`profile.py`, its own group, `HOME=/tests`): assignments and `export`s in it are in effect at the first prompt (`echo $X`, `printenv`); `PATH=$PATH:...` (expansion works); a `cd` in it sticks; a bad line is
  reported and the rest still run; no file / a directory named `.profile` / an empty file are all silent or noted as above and the shell starts; output from a command in it appears before the first prompt (harness reads the
  boot log); `./script`-style isolation is *not* applied (it is not a scope); `PS1` set there shows at the first prompt.
- Docs: `shell.md` "Starting up" (the order is now environment, `$HOME`, profile, prompt), `filesystem.md` (`/root/.profile`), `tests.md` (`PROFILE`).

## Step 11 -- docs, roadmap, sweep
`docs/shell.md` (variables, assignment, `export`, expansion, `$?`, the retired line, `PATH`, `PS1`), `docs/progs.md` (`env`, `printenv`, `date`/`stat` and `$TZ`), `docs/launching_programs.md` (`envp` layout
and `x2`), `docs/filesystem.md` (`/etc/environment`, `/root`), `docs/tests.md` (environment-file injection, `ENVIRONMENT`), repoint `docs/*` from `r16_brk` to `r17_env`, `ROADMAP.md` Stage 17
"As built" (and the `/etc/environment` + `/root` + `.profile` decisions; a persistent `/root` mount is *not* part of this stage and is raised separately), `just check-docs`, `just lint`. Regression: `r16_brk`, `r15_large_binaries`, ... `r11_busybox` (userlib and `progs` are shared:
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
