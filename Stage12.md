# Stage 12: a real shell, with pipes -- `r12_shell` (full plan)

`ROADMAP.md` carries the summary of this stage (goal, features, demo, and how it connects to later stages);
this file is the full plan: every Step, its tests, the state the stage ends in, and the decisions behind it.

| Step | What | Status |
|---|---|---|
| 0 | Scaffold `r12_shell`, test infrastructure, this document | done |
| 1 | Error handling, shared `abi`, launch by path, heap growth, small cleanups | done |
| 1b | Code review and reorganization of what Steps 0-1 produced | done |
| 2 | Console write path (streaming UTF-8, fewer flushes, segfault message) | done |
| 2b | Unicode console (Unifont glyphs, wide cells) | done |
| 3 | Turn the MMU on for real; user stack mapping and guard | done |
| 4 | One line-discipline module | done |
| 5 | Eval loop out of IRQ context (token queue) | -- |
| 6 | Working directory and the shell-state frame stack | -- |
| 7 | Lexer, `run_line`, bash wording | -- |
| 8 | Redirection (`<`, `>`, `>>`, `2>`, `2>>`, `2>&1`) | -- |
| 9 | Scripts and scopes | -- |
| 10 | `mkdir`, `rm -r`, `mv` | -- |
| 11 | Pipes via temp files | -- |
| 12 | Line editing and history | -- |
| 13 | Docs, roadmap, full regression | -- |

## Goal
Turn Stage 10's launcher into a shell worth typing at -- line editing with history, `cd`/`pwd`/`mkdir`/`rm`/`mv`,
redirection, scripts with correct scoping, pipes -- and boot straight into its read-eval loop, the role a kernel's
`init` plays. Reviewing Stages 9-11 also turned up problems in the existing system that the shell shouldn't be built
on, so the stage has two phases: **Phase 1 (Steps 1-5) fixes and restructures what exists**, **Phase 2 (Steps 6-12)
builds the shell**, with Step 0 (scaffold) before and Step 13 (docs and regression) after. The shell stays
kernel-resident, as the roadmap says; the connections to a later userspace `sh` are recorded below.

## Ground rules
- Each `rNN_` directory is a self-contained snapshot (the repo shows the progression; Stage 24 will eventually be
  copied out on its own). `r12_shell/` started as a copy of `r11_busybox/`; **r09-r11 are not touched** and there is
  no shared kernel crate across stages.
- `user/userlib`, `user/progs` and the new `user/abi` are shared by every stage: changes there are additive and
  backward compatible. **`user/` holds only the common binaries that build up the core-utils set; tests belong to this
  stage** (`disk/tests/`, `test/progs/`).
- Every Step ends with `just test` green in `r12_shell/` (extending `test/run_tests.py`) and the r09-r11 demos still
  building against the modified `user/` crates. Commits only when asked; each Step is a natural commit boundary.
- ROADMAP.md edits touch only unimplemented stages (12, 13, 16, 19, 22, 23, 24); Stage 9-11 text stays as written.
- QEMU output capture in tests and manual runs: background + redirect to file + sleep + kill + cat (not
  `mon:stdio | timeout | head`, which loses buffered output).

Facts confirmed while planning (r11): `vectors.s` already routes `irq_el0_64` -> `irq_handler` with
`kernel_entry/exit`; `blk.rs` waits with `wfe` + `peek_used()` (works with IRQs masked or not); each crate has
its own `justfile` (BIN, disk label, `test` recipe) and `build.rs`; the test harness is a Python `Session` class
(`type`, `run`, `wait_prompt`, `check`) driving QEMU's monitor `sendkey`; the kernel's token layer already resolves
the full US layout, so every shell character can be typed through `sendkey`.

## Source layout (`r12_shell/src/`)
r11's flat 28-file layout became a hierarchy after Step 1, before Step 2 adds more modules. The rule is that a
module uses ones at or below its own level: `arch`/`platform` (the CPU and the board) -> `drivers` (device
protocols) -> the services built on them, `fs`, `console`, `keyboard` -> `exec` (programs) -> `syscall` -> `shell`.
**Convention:** every module is a directory, and its `mod.rs` is the module's main file, the way `__init__.py` is
in Python -- the module's own definitions (if it has any) plus its `pub mod` lines, so a module lives in one place
and its main file is recognizable by name. The children are the other files in the directory. (`console/mod.rs` is the
console; `syscall/mod.rs` is the dispatcher; `shell/mod.rs` is the read-eval loop.) Pure modules (P) stay next to
their subsystem and are pulled into `hosttests/` by path.

```
src/
├── main.rs            crate root: kernel_main, IRQ dispatch (irq_handler), panic handler, heap
├── util.rs            static_mut_ref!/static_ref! macros                                  (was utils.rs)
├── arch/              mod.rs; boot.s, vectors.s, context.s (was process.s); mmu.rs; gic.rs (gic_setup/gic_enable, from main.rs)
├── platform/          mod.rs
│   ├── base_addresses.rs   the platform's address map: DTB-discovered addresses plus fixed constants (user window included)
│   ├── uart.rs             PL011 driver, UART0, and the transcript mirror (uart_write/uart_ensure_newline)
│   └── globals.rs          the kernel's global device statics                              (was devices.rs)
├── drivers/           mod.rs
│   └── virtio/        mod.rs: find_mmio_transport (shared slot probing)
│       └── hal.rs (was virtio_hal.rs)   blk.rs   gpu.rs (returns a FramebufferInfo)   input.rs (was keyboard.rs: raw events)
├── fs/                mod.rs: find_entry_checked, read_file_checked
│   └── blkio.rs (was fat_io.rs; also the VOL static)   files.rs (open-file table, lookup, resolve; owns MAX_OPEN_FILES)
├── console/           mod.rs: Console, show_row, FG/BG
│   └── framebuffer.rs (the Framebuffer struct, out of console.rs)   font.rs (glyph_for, widths)   cells.rs (P)   utf8.rs (P)
├── keyboard/          mod.rs
│   └── keymap.rs   tokens.rs   events.rs (was input.rs)   line.rs (LineBuffer, plus the LINE/INPUT_ROW statics)   stdin.rs
├── exec/              mod.rs
│   └── argplan.rs (P)   elfparse.rs (P)   usermem.rs (P)   elf.rs (maps the window)   process.rs
├── syscall/           mod.rs: dispatch and the fault path
│   └── fd.rs          the fd table
└── shell/             mod.rs: PROMPT and handle_keyboard_irq (the loop, for now)
    └── launch.rs (find_program, launch, report)   argv.rs
```

Cycles found and removed by the move: `fd` <-> `files` (the open-file limit now lives in `fs::files`, and `fd` sizes its
table from it); `keyboard::stdin` <-> `syscall::fd` (the UART transcript helpers moved down to `platform::uart`); `gpu` ->
`console` (the driver now returns a plain `FramebufferInfo` and the console wraps it). The duplicated `EXEC_BIT`,
`READ_ONLY_BIT`, `VOLUME_LABEL_BIT`, `NAME_MAX` and `DIRENT_SIZE` constants were replaced by `abi::fs`'s.

**Prompt drawing.** `console::show_row` is the drawing primitive (a prefix plus text on a row, with the sliding window).
`keyboard/` owns editing and echoing a line -- the buffer, the row it sits on, the redraw after each edit -- given
whatever prefix to draw (`""` for a program's `read(0)`). `shell/` owns the prompt itself: the `PROMPT` string and when
a fresh one is drawn. Today `handle_keyboard_irq` does both jobs and lives in `shell.rs`; Step 4 splits it along that
line. `keyboard/` may call `console` (echo); `console` never calls back.

The Step descriptions below were written against r11's file names; `Source layout` above is the current map (for
example `line.rs` is `keyboard/line.rs`, `fd.rs` is `syscall/fd.rs`, `files.rs` is `fs/files.rs`, and Step 4's new
`line_discipline.rs` will sit in `keyboard/`).

---
## Step 0: scaffold `r12_shell` (done)
- `rust/r12_shell/` copied from `r11_busybox` (justfile `BIN`/disk label `R12SH`, Cargo package name, test script);
  build products (`target/`, `*.elf`, `disk.img`, `disk/bin/*.exe`) not copied.
- **Disk image: 64 MiB FAT16** (`folder_to_img.sh disk.img 64M R12SH ...`). FAT16 stays valid to ~2 GiB, so no format
  change and no FAT32; the kernel reads capacity from the device, so only the justfile argument changed. FAT16's root
  directory is a fixed 512 slots (long names use several), so programs, fonts and temp files live in subdirectories
  (`bin/`, `fonts/`, `tests/`, `tmp/`). Built image: 65 MB free, `fsck.fat -n` clean.
- **`.gitignore`:** generated `disk.img` is ignored from this stage forward, with explicit negations keeping r06-r11's
  tracked images visible; `disk/tests/*.exe` and `disk/tmp/` ignored too. `just run`/`just test` depend on `just disk`,
  so a fresh checkout builds the image on first use.
- **Test scaffolding**, so every later Step adds its tests as it goes: `test/progs/` (Cargo package `testprogs`, skeleton
  `probe`) built and staged as `disk/tests/<name>.exe` by `just disk`; static fixtures in `disk/tests/` (the utility-demo files `hello.txt`, `data.bin`, `docs/example.txt` moved there
  from the disk root, plus `notes.txt`); `just disk` creates the empty pipe directory `disk/tmp/`; the harness split into `test/harness.py`, `test/run_tests.py` and `test/cases/` (the r11 utility cases became
  `cases/core_utils.py`, using `tests/...` paths; each Step adds a module) with additions (panic
  detection on every wait, sparse image copy, extended `KEY_NAMES` and named keys, `screendump()`, final `fsck.fat -n`
  check); `hosttests/` crate skeleton; `just test-host` / `just test-qemu` / `just test`; `test/README.md`.
- **Docs:** this file, and ROADMAP.md's Stage 12 reduced to a summary with the forward-connection edits.
- **Done:** `just test` passes; the only expectation changes from r11 are the paths (fixtures live under `tests/`) and the root listing, which is now
  `bin`, `fonts`, `tests`, `tmp`; `git status` shows nothing under `rust/r06..r11`.

## Phase 1: fixes and restructuring of existing functionality

### Step 1: error handling, shared ABI, small cleanups (done)
- **ELF load never panics** (`elf.rs`, `main.rs::launch`): `load()` returns `Result`; validate *every* header and
  segment (incl. `p_offset+filesz` in file bounds, segment within window, no overlaps that panic the mapper) before
  copying/mapping anything. `launch` reports `name: cannot execute: Exec format error` (bash's wording, `ENOEXEC`). Reason: `chmod +x` on any file (Stage 11)
  makes the old "build-time mistake" panic reachable. `launch`'s remaining `.expect`s (open/read errors) become
  reported errors.
- **`abi` crate** (`rust/user/abi/`, `no_std`; modules `syscall`, `errno`, `fs`): syscall numbers, errno values,
  dirent layout/`DIRENT_SIZE`, FAT attr bits. Used by r12's kernel and by `userlib`/`progs` (replaces `progs::errmsg`'s magic numbers and the
  "in sync by convention" copies). Existing values unchanged so r09-r11 keep working.
  r12 kernel behavior changes: unknown syscall -> `ENOSYS`(-38) (was `-1` = EPERM); bad user pointer -> `EFAULT`(-14) (was EBADF).
  New errno values added for later Steps (Linux numbers, with `errmsg` text): `ENOEXEC` 8, `EFAULT` 14, `EEXIST` 17,
  `ENOSPC` 28, `ENAMETOOLONG` 36, `ENOSYS` 38, `ENOTEMPTY` 39.
- **Launch by path:** a command word containing `/` runs that path (exec bit required, root-relative until Step 6 makes
  it cwd-relative); a bare name keeps the bin/ lookup (bare name, then `name.exe`). Needed first so the tests under
  `/tests/` can run; Step 9's `./script` builds on the same path. Classification (POSIX/bash `ENOEXEC` model): a file with
  ELF magic is loaded as a program (a malformed one -> `cannot execute: Exec format error`); at this Step *any*
  non-ELF file is reported the same way. Step 9 then adds bash's fallback for non-ELF files that look like text.
- **`push_cstr_array(sp, &[&str]) -> (sp, array_ptr)` helper** extracted from `run_program`'s argv writing (Stage 16
  will reuse it for `envp`; the env-aware entry macro and `x2` forwarding stay deferred to Stage 16).
- **Kernel heap growth (1 MiB -> 16 MiB, see analysis below):** raise `HEAP_SIZE` in `main.rs`; the heap is a static in
  `.bss`, so `mmu.rs` maps it automatically via `__data_start..__kernel_end`; update `main.rs`'s comments
  since the image grows past `0x41000000`. (`user/progs/link.ld`'s comment about the kernel image was written for the smaller one
  in r09-r11; it was reworded in Step 3 to cover both, since the file is shared by every stage.)
- Small items: `MAX_FDS`(16) vs `MAX_OPEN`(8) made coherent (EMFILE behavior documented/tested); `files::close_all`
  no longer runs twice per launch; stale comments refreshed. The boot-time exec-bit rewrite of `bin/*` stays
  (documented mtools workaround).
- **Tests:** `chmod +x` a text file, run it -> `cannot execute`, prompt survives; truncated/garbage ELF likewise;
  a small test program issuing an unknown syscall gets ENOSYS and one passing a bad pointer gets EFAULT.

**As built (differences from the plan above, and what was added):**
- The ELF loader is split: `elfparse.rs` is pure (no kernel dependencies) and validates the whole file -- header,
  program header table, every segment against the user window and the file, entry point inside a segment -- before
  `elf.rs` copies or maps anything; both are `Result`-based (`ElfError`). The pure half has 10 host tests
  (`just test-host`), including "every truncation of a valid ELF is refused" and wrap-around offsets.
- `argplan.rs` (pure, host-tested) plans the argv layout; `process.rs` has `push_cstr_array` (writes it) and the
  `prepare`/`run` split (`run_program` = both). **`argv[argc]` is now `NULL`** (the C convention; Stage 10's array
  had no terminator), and an argument list over `ARG_MAX` (128 KiB) is refused up front (`E2BIG`, checked before the
  load) instead of running into the program's memory.
- `abi` is split into three modules used by their full path: `abi::syscall` (numbers, including `SYS_CHDIR` 49,
  reserved and unimplemented), `abi::errno` (values plus `errmsg`, so `progs::errmsg` and the kernel share one
  wording) and `abi::fs` (`O_*` flags, the `getdents` record, FAT attribute bits). The kernel has no `errno.rs` of its
  own any more: it imports `abi::errno::...` directly. `userlib` re-exports the syscall/`fs` names flat, so programs
  still write `userlib::SYS_WRITE`. Each module's tests pin every value r09-r11's own copies use.
- Directory lookups no longer panic on a read error: `find_entry_checked` returns `Result<Option<_>>` (`EIO` for an
  unreadable directory, `None` for a miss) and is the only lookup function. Boot-time callers (`kernel_main`,
  `read_font`) `expect` each case separately, so an I/O failure and a missing file give different messages.
  `files::lookup(path)` finds an
  entry without opening it, and `launch` is built on it (`bin/<name>`, `bin/<name>.exe`, or the path as typed).
- `launch` refuses a directory (`Is a directory`), a file over `MAX_PROGRAM_SIZE` (8 MiB, half the heap),
  a missing exec bit (`not executable`, wording unchanged until Step 7) and anything `elf::load` rejects
  (`cannot execute: Exec format error`).
- **Known quirk, resolved in Step 6:** with no working directory yet, every path -- a command word containing `/`,
  and every path a program opens -- is resolved from the disk's root, so `tests/probe.exe` and `/tests/probe.exe`
  are the same file and `.`/`..` match nothing (the same holds in Stage 11's `open`). Step 6's working directory and
  `abspath` make a path without a leading `/` relative to the working directory and handle `.`/`..`; its
  tests cover the change. (Bare names still search only `bin/`, by design, until a later stage adds `$PATH`.)
- `fd::MAX_OPEN_FILES` (13) is the single limit for `files.rs` and the fd table; `reset_for_launch` no longer calls
  `close_all` (`end_launch` already has).
- `test/mkfixtures.py` (run by `just disk`) derives `bigpad.exe` and seven malformed ELFs from `echo.exe`/`hello.exe`
  into `disk/tests/`; `probe` has `sys-unknown`, `bad-ptr`, `fds` and `args`; `cases/step01_launch.py` holds the
  QEMU cases (T1.1-T1.7; T1.2-T1.4 as seven malformed files instead of three).
- Compatibility (T1.9) was checked by building r09's and r10's disks and running r11's `just test` against the
  modified `user/` crates: all pass. Those recipes regenerate the tracked `disk.img`/`disk/bin` files there, so
  restore them afterwards (`git checkout -- rust/r09_userspace rust/r10_repl rust/r11_busybox`).

### Step 1b: code review and reorganization (done)
An unplanned pause between Steps 1 and 2: the crate as it stood (r11's code plus Step 1's) was read through and
reorganized before Step 2 adds more modules. No behavior change except where noted; `just test` stayed green throughout,
and r09-r11 still build (and r11's `just test` passes) against the changed `user/` crates. The plan for later Steps keeps
its r11-era file names; "Source layout" above maps them.

**Reorganization**
- **Source layout:** the flat 28-file crate became the hierarchy in "Source layout", each module a directory with a
  fat `mod.rs`. It removed three dependency cycles (`fd`<->`files`, `stdin`<->`fd`, `gpu`->`console`) and the duplicated
  attribute/dirent constants. `build.rs` scans `src/` recursively for the assembly.
- **`abi` split into modules** (`abi::syscall`, `abi::errno`, `abi::fs`), used by full path; the kernel's `errno.rs` is gone.
- **`userlib` split into three modules**, re-exported flat so no program changed: `syscall` (the raw `svc` and the
  `syscall!` macro, crate-private), `io` (`read`, `write`, `open`, `close`, `getdents`, `DirEnt`, `chmod`) and `process`
  (`exit`, exit statuses, `entry!`/`entry_with_args!`, `Args`, the panic handler and `start.s`). `exit` is the one syscall
  outside `io`, because it belongs to the program's own life.
- **Virtio drivers** share `find_mmio_transport(slots, device_type)` instead of three copies of the slot-probing loop,
  assuming one device per type (as on this machine). The per-slot size is `VIRTIO_SLOT_SIZE`, distinct from the
  whole-window `VIRTIO_MMIO_SIZE` in `platform::base_addresses`.

**Behavior and API changes**
- **The exit status travels with the jump.** `resume_kernel(code)` leaves the status in `x0`, so it is `enter_el0`'s return
  value (a `longjmp` value) and `run` takes it from the `asm!` output. The `EXIT_CODE` global, `set_exit_code` and the
  "never set" state are gone. The `exit` syscall masks the status to 8 bits as POSIX does (`exit 300` reports `exit 44`);
  `probe exit N` and five QEMU cases cover it.
- **Renames:** `Prepared` -> `PreparedProgram`; `argstack` -> `argplan`; `files::walk` -> `resolve` (a directory from a
  slice of components -- Step 6's path handling is `abspath` on top of it); `read_file_to_vec` -> `read_file_or_panic`
  (paired with `read_file_checked`).
- **One directory-lookup function.** The panicking `find_entry` wrapper was removed; boot-time callers use
  `find_entry_checked` with two `.expect`s, so an I/O error and a missing file give different messages.
- **One open-file limit:** `files::MAX_OPEN_FILES` (13), with the fd table sized from it; the private `MAX_OPEN` alias was removed.
- **Cleanups:** `arch/mmu.rs`'s one-line `region()` wrapper was removed; the `find` functions of `blk`/`gpu`/`input` shrank
  to a few lines each.

**Comments corrected or rewritten** (several were stale from r09-r11, or wrong):
- `arch/vectors.s` and `arch/context.s`: `kernel_entry` is the entry into the kernel from EL0; every exception ends one of
  three ways (`kernel_exit`, `resume_kernel`, `unexpected_exception`); `resume_kernel` abandons the trap frame by resetting SP,
  as `longjmp` does.
- `arch/mmu.rs`: what a level-1 root is (512 entries of 1 GiB, a 39-bit space) and what the TLB-invalidate sequence does.
- `exec/process.rs` and `syscall/fd.rs`: closing files at program exit finishes writes in progress; it is cleanup, not a
  save of the program's own state.
- `keyboard/stdin.rs`: `PENDING`/`PENDING_POS` are the current line and how much of it `read(0)` has handed out.
- `platform/base_addresses.rs`: the virtio window is four 4 KiB pages (16 KiB), not "2 KiB"; empty slots read back device ID 0.
- `shell/mod.rs`: `handle_keyboard_irq`'s long doc comment was condensed to what is still true.
- `abi::fs`: `O_RDONLY`/`O_WRONLY` document the `flags` argument of `SYS_OPEN`.

**Decided during the review, not changed**
- `platform/base_addresses.rs` stays the platform's full address map (constants for what is fixed, `BASE_ADDRESSES` for what
  the device tree reports, `USER_*` included); the fixed virtio window stays hard-coded rather than derived from the
  discovered slots.
- `.`/`..` and `ls -a`: `list` keeps skipping `.`, `..` and the volume label. If `ls -a` is added, the Unix way is to
  return every real entry from `getdents` and filter in `ls`, which then makes every other directory consumer (notably
  Step 10's `rm -r`) skip `.` and `..` itself.

**Carried into later Steps:** the segfault message reaching the console (Step 2, above); `abspath`/`resolve` (Step 6).

### Step 2: console write path (`fd.rs`, `userlib`/`progs::Fd`) (done)
- Replace `from_utf8().unwrap_or("<invalid utf8>")` with a **streaming UTF-8 decoder** (`utf8.rs`, pure) in the kernel's
  Console write path: it keeps partial-sequence state between `write` calls (so a multibyte character split across
  `cat`'s 4096-byte chunks decodes correctly) and yields `char`s; an invalid byte yields U+FFFD and the decoder
  resynchronizes, instead of blanking the whole chunk. The decoder state is one static (single resident program; it
  becomes per-process state with Stage 19). The UART mirror keeps sending the raw bytes, so a split character is
  intact on the serial transcript. Until Step 2b the `char`s still go through the existing `cp437::unicode_to_cp437`
  (U+FFFD, like any unmappable character, shows as `?`); Step 2b replaces that mapping with a Unicode font behind a
  single `glyph_for(char)` function, so nothing above it changes.
- Fewer flushes: a `write!` makes one `write` syscall per fragment, and each syscall costs a GPU flush. So `Fd(1)` (stdout)
  buffers small `write_str` fragments in one static in `progs` (single thread, one user of fd 1) and sends them in one
  `write`: on newline, at exit, before a blocking `read`, and **before every other write** -- stderr's included (C++'s
  tied-`cerr` rule, so `cmd > f 2>&1` keeps exact program order even for a partial line). stderr is unbuffered.
  The buffer lives in `userlib` (`write_stdout`/`flush_stdout`, 512 bytes; `exit` flushes) because `exit` and `read` are
  there, and `progs::Fd(1)` uses it; `userlib::write` flushes it first, so raw `write(1, ..)` calls stay in order too. Known cost, same as C's stdout: a fault loses the unflushed partial line (a panic flushes first). The
  kernel already flushes the GPU once per write syscall (not per fragment); UART mirroring unchanged. A `testhooks` cargo
  feature (enabled by `just test`, off for `just run`) keeps a GPU-flush counter printed on the UART at program exit,
  so the flush-count test (T2.5) is deterministic.
- **Show the segmentation fault on the console** (raised in the Step 1b review). Today the fault handler
  (`syscall/mod.rs`) writes `Segmentation fault (address ..., ESR_EL1 ...)` to the UART only, so the console shows just
  `exit 139`. Its reason for that ("writing through `Console` would move the cursor without `INPUT_ROW` being
  resynced") no longer holds: `handle_keyboard_irq` resyncs `INPUT_ROW` from the console cursor after `launch`
  returns, whoever wrote. So extract the console arm of `FileDescriptor::write` into a helper (`console_write(bytes)`
  in `syscall/fd.rs`, mirroring to the UART itself) and have the fault path call it after starting a fresh line if the
  cursor is mid-line, as `report` does; then drop the stale comment. The serial transcript is unchanged, so the
  `crash` case in `core_utils` still passes; add a screendump check that the message is on the display.
- **Tests:** `cat` of a binary fixture: serial transcript matches expectations byte-for-byte; a UTF-8 fixture larger
  than 4096 bytes with a multibyte char on the boundary; timing sanity on a long `write!` loop (no per-fragment flush).

**As built.** `console/utf8.rs` (WHATWG decoder, 8 host tests incl. every split point) feeds `console_draw` in `syscall/fd.rs`
(one GPU flush per call); `console_write` adds the raw-byte UART mirror and is what the fault path calls
(`console_start_line` first, which also ends any half-written character). `end_launch` turns a character left incomplete into
one U+FFFD. The `testhooks` feature and `just build-test` (`r12_shell-test.elf`, what `just test-qemu` runs) print
`[testhooks] console_flushes=N` when a program ends; the harness strips those lines from transcripts and exposes them as
`Session.flush_counts()`. New: `probe frag|frag-raw|interleave`, fixtures `binary256`, `utf8-boundary.txt` (a two-byte
character straddling offset 4096) and `utf8-line.txt`, `cases/step02_console.py`. Not done here (Step 8): the `> f 2>&1`
form of T2.6 -- the console-order form passes now. Noted, unchanged: after a program whose output does not end in a
newline, the serial log shows the next prompt directly after it (as in r11); only the display starts a fresh line.

### Step 2b: Unicode console (`console/font.rs`, `console/mod.rs`, `keyboard/line.rs`, `Cargo.toml`) (done)
Decided in the Step 2 discussion: instead of converting Unicode to a private codepage, draw Unicode with GNU Unifont
through the `unifont` crate (`no_std`, no dependencies, MIT; the font data is Unifont's own, dual-licensed GPLv2+ with
the font-embedding exception / OFL 1.1).
- **One font, compiled in.** `glyph_for(c: char)` (in `font.rs`, the one place that knows about fonts) returns
  `unifont::get_glyph(c)`, or U+FFFD's glyph for a character with no glyph (everything above U+FFFF included) and for
  any control character the console doesn't interpret. The Spleen font is dropped: `disk/fonts/spleen.raw`, `FONT_DATA`,
  `read_font` in `main.rs` and `cp437.rs` are deleted, so the boot no longer reads a font from disk. `disk/fonts/` stays
  (later Steps' tests use it as a second directory) and its `NOTICE` becomes Unifont's licence text. The cell size stays
  8x16. ASCII therefore looks different from r11 (Unifont's shapes); every screendump baseline in Steps 3-12 is taken
  after this Step.
- **The console gets a cell grid.** Today it is pixels plus a cursor, with no record of what is where (see
  `scroll_up`'s comment), so it can't know that the character before the cursor was wide. Add one byte per cell
  (`Narrow`, `WideLeft`, `WideRight`; blank = `Narrow`), scrolled with the pixels in `scroll_up` and reset by
  `clear`/`clear_row`. Wide glyphs are two cells: the width comes from the glyph itself (`is_fullwidth()`: 16 px = two
  cells, 8 px = one), with no East Asian Width table. A wide glyph that would start in the last column wraps to the next
  line first. Drawing over one half of a wide glyph blanks the other half (no orphaned half-glyphs).
- **Width-aware editing** (raised in review): Backspace and every place that counts columns count *cells*.
  - `write_char` gains **BS (0x08)**: it moves the cursor back one *character* -- two cells if the previous cell is a
    `WideRight`, else one, stopping at column 0 -- and does not erase (a program erasing sends `\b\b  \b\b` for a wide
    glyph, as with any terminal).
  - `show_row`'s budget (`prefix.chars().count()`, `line.chars().count()`) becomes a sum of cell widths, and its sliding
    window never starts in the middle of a wide character.
  - `LineBuffer`'s Backspace already removes one whole `char` (never half a UTF-8 sequence); Step 12's cursor-aware editor
    tracks widths so Left/Right/Delete move by character and the cursor is drawn at the right cell. Today a typed line is
    always ASCII (US-only keymap), so this is exercised by host tests and by history/paste-like sources, not by keystrokes.
- **Zero-width code points draw nothing and take no cell:** U+200B-U+200F, U+2060, U+FE00-U+FE0F, U+FEFF (ZWJ,
  joiners, variation selectors, BOM). Combining marks draw as ordinary standalone glyphs in their own cell.
- **Known limits (documented in `docs/`):** BMP only -- the crate has no astral plane, so emoji and other characters above
  U+FFFF draw U+FFFD (deliberate: the astral plane is a small audience for a converter and a font file format); no
  emoji sequences, bidi or complex-script shaping (Arabic and Indic scripts show as isolated glyphs in logical order).
  Unifont's table is compiled into the kernel image's `.rodata` (the crate's `get_storage_size()` reports the exact size;
  record it here after the first build -- the image must stay below the `0x44000000` user window). The crate's lookup is
  a linear scan over its code-point ranges; add a small cache for ASCII only if a full-screen `cat` measures slow.
- **Upgrade path if the astral plane is wanted later:** BDF and PCF can only hold Plane 0 (unifoundry.com/unifont), so
  they are not the source. Unifont publishes the astral glyphs (`unifont_upper`) as `.hex` -- plain text, one glyph per
  line, `CODEPOINT:HEXBITMAP` (32 hex digits = 8x16, 64 = 16x16) -- the same format the `unifont` crate's `build.rs`
  already parses. A host-side script would merge the two `.hex` files into a range-indexed binary file, records
  `(first, count, data_offset, wide)`, binary-searched; only `glyph_for` changes.
- **Effort budget:** no key can type a non-ASCII character (US-only keymap), so this is all edge cases -- the goal is only
  that Unicode text from files and programs displays gracefully or degrades to U+FFFD, not a complete text stack.
- **Tests:** see T2b below (host: `glyph_for` classification, the cell-grid and Backspace rules, the width-aware
  `show_row` window; QEMU: CJK fixture, wrap at the last column, `echo é`, invalid bytes, `cat binary256`).

**As built.** `console/font.rs` (`glyph_for`, `cell_width`, `is_zero_width`, `tail_window`) and `console/cells.rs`
(`CellGrid`: `place`, `back`, `fits`, `scroll_up`) are pure and host-tested (14 tests, with the `unifont` crate as a
`hosttests` dependency); `Console` owns a `CellGrid` and draws through `draw_glyph` (16-px-wide glyphs span two cells),
`write_char` handles BS, zero-width code points and wrapping a wide glyph whole, and `show_row` measures in cells with
`tail_window`. Removed: `cp437.rs`, `FONT_DATA`/`read_font`/`Font` (the `Console` lost its lifetime parameter),
`fs::read_file_or_panic` (its only caller was the font read), `disk/fonts/spleen.raw` (`NOTICE` now describes Unifont).
Measured: the crate's tables add a read-only segment of about 1.9 MB (`.rodata`, 0x1e5520 bytes); the kernel image now ends
ends at `0x41712080` (about 23 MiB after Step 3's page alignment), far below the user window. Display is 640x480 = 80x30 cells (the fixtures assume 80 columns). Tests:
`cases/step02b_unicode.py` (a CJK line is exactly six cells; a wide glyph at column 80 wraps whole and leaves the last
cell blank; `é` differs from U+FFFD while an emoji, an invalid byte and a control character each draw exactly U+FFFD; a
zero-width joiner leaves `a<ZWJ>b` identical to `ab`; `probe bs-wide` -- backspace over a wide glyph then `X` equals `X`
alone), fixtures `cjk.txt`, `wide-wrap.txt`, `unicode-mix.txt`, probe subcommand `bs-wide`.
**Wrapping follows xterm** (added after reviewing a screenshot of a long `cat`): a glyph that ends in the last column leaves the
cursor there with a *wrap pending* (`cells::Cursor`), and the wrap -- and any scroll -- happens when the next glyph arrives.
CR, LF, BS, tab and explicit positioning clear the flag without wrapping, so a full row followed by `\n` leaves no blank row,
and `cursor()` reports the last column while pending, which the "am I mid-line?" checks (`report`, the fault path, the
prompt resync) read correctly. `show_row` lost its reserved trailing column and uses the full width. Deliberately *not* done:
hanging or swallowing a space that lands in column 0 after an automatic wrap (xterm prints it there; so do we), and
word-wrapping (it would break the cell model). Tests: 10 host tests on `Cursor`; QEMU -- `full-row.txt`, `full-row-space.txt`
and a typed command exactly as wide as the row. The ASCII screendump baseline
for later Steps is simply the Unifont look from here on (no baseline files exist yet).

### Step 3: turn the MMU on for real; user stack mapping and guard (`mmu.rs`, `elf.rs`, `usermem.rs`, `link.ld`) (done)
- First task is **verification**: `elf.rs` maps only `PT_LOAD` segments, yet argv is written at `USER_BASE+USER_SIZE`
  downward and programs run; find/prove where those pages get mapped (aarch64-paging behavior or a probe), and
  confirm nothing else is being relied on by accident.
- Make the stack an explicit mapping (size chosen generously, per project practice) and leave an unmapped guard
  between it and `.bss`. Also unmap/revoke the previous program's extent so stale mappings can't leak between
  loads (a small piece of Stage 17's shrink-on-load concern limited to what this step touches; the full variable
  window remains Stage 17).
- **Tests:** an infinite-recursion program -> `exit 139` with no corruption of neighbors; a program that touches the
  guard faults; existing programs (incl. `tail` with its 512 KiB static buffer) still run.

**What the verification found (T3.5).** The plan assumed the stack pages were mapped by something. Nothing maps them, and
neither does anything else: **the MMU has never been on**, in any stage. `SCTLR_EL1.M` is 0 (read back with a probe), `TCR_EL1`
was never configured beyond `EPD1`, and `aarch64-paging`'s `activate()` only writes `TTBR0_EL1`. Every stage since Stage 9 has
built its page tables and set `TTBR0_EL1` without turning translation on, so the RX/RO/XN split, the EL0-only window and the
"unmapped guard gaps" were never enforced -- EL0 could read and write kernel memory and the UART, and `overflow` ran through its
own code and into the kernel. The programs worked because with translation off every address is its own physical address. Stages
9-11 (`r09`-`r11`) are unchanged and still have this; whether to back-port the fix is open (see below).

**As built.**
- `arch/mmu.rs` now sets `TCR_EL1` (T0SZ 25 = 39-bit VA for the level-1 root, 4 KiB granule, write-back caching of walks,
  inner shareable, `IPS` from `ID_AA64MMFR0_EL1`, `EPD1`) and `SCTLR_EL1` `M | C | I`, after `activate()`.
- `link.ld` (kernel and both user scripts) aligns every region boundary to 4 KiB. Permissions are per page, and `.rodata` starting
  in `.text`'s last page made the vector table non-executable on the first IRQ. `elfparse` refuses an executable whose segments
  share a page (`SegmentsShareAPage`); the user scripts and `build.rs` (`rerun-if-changed=link.ld`) were updated.
- Window layout (`platform/base_addresses.rs`): image up to `0x440f0000`, unmapped gap, a 64 KiB guard, a 1 MiB stack ending at
  `0x44200000`. `elfparse` bounds segments to the image part.
- `elf.rs` rewritten: each load unmaps the whole window, maps each segment's pages writable, zeroes them (no leftovers from the
  previous program) and copies, maps and zeroes the stack, then locks each segment to its own permissions (RX / RO / RW+XN);
  `dc cvau` + `ic ialluis` make the code visible to instruction fetch. Kernel writes obey page permissions too, which is why
  the write-then-lock order matters.
- `exec/usermem.rs` (pure, host-tested) records what is mapped and whether it is writable. `syscall/fd.rs::validate` checks
  every user pointer against it: with the MMU on, the kernel itself faults on an unmapped or read-only page, and a kernel fault is a
  panic, so a pointer into the guard, the gap or the program's own code is now `EFAULT`.
- Tests: `cases/step03_stack.py`; probe subcommands `poke`, `poke-w`, `user-ptrs`, `sp`, `stack`; test program `overflow`;
  fixtures `elf-inguard.exe`, `elf-instack.exe`, `elf-sharepage.exe`; the `crash` case's expected `ESR_EL1` is now `0x92000004`
  (a translation fault; it was `0x92000000`, an address-size fault, with translation off).
- **Hardening on top (decided after the review):** `SCTLR_EL1.WXN` (no page is both writable and executable), `SA`/`SA0`
  (misaligned stack pointers fault at EL1/EL0) and **PAN** when the CPU has FEAT_PAN (`ID_AA64MMFR1_EL1`): with `SPAN` = 0 every
  exception entry sets `PSTATE.PAN`, so the kernel faults on any access to a user-accessible page unless it asked for it with
  `mmu::user_access()` (a guard that clears PAN and restores it on drop). Held by the loader, `prepare`'s `argv` writes, and the
  five syscalls that take a user pointer. Boot prints `MMU hardening: ...`, and the harness checks it. Verified that PAN bites
  by removing the guard from `write`: the first user-buffer access took a permission fault (`ESR_EL1 0x9600000f`) and panicked
  the kernel. Deliberately not enabled: `SCTLR_EL1.A` (alignment faults), since Rust's `read_unaligned` (`elfparse.rs`) compiles to
  plain unaligned loads.
- **Identity mapping stays** (decided): kernel and user both keep virtual = physical addresses, in one table, as long as
  possible. What would change that is several processes at once: a kernel in the high half (`TTBR1`) with only `TTBR0` swapped per
  process, and user virtual addresses independent of physical ones, so two programs can share one link address (today every
  binary links at `0x44000000`, which is why only one can be resident). Both are prerequisites for Stages 17-19, not for Stage 12.
- **Decided: `r09`-`r11` are left exactly as built.** They are complete stages and the repo shows the progression; ROADMAP's Stage 9
  carries a warning that the MMU was never activated there, and its Stage 12 summary carries the matching note that this stage is
  where it is.

### Step 4: one line-discipline module (`line.rs`, new `line_discipline.rs`) (done)
- Replace the two duplicated implementations (`handle_keyboard_irq`'s draw/finish/UART-mirror logic and
  `stdin::read_line`) with one module owning: `LineBuffer` feeding, current input row (`INPUT_ROW`), echo/redraw,
  newline on finish, UART transcript. Both the prompt and `read(0)` call it. Behavior identical to r11; no
  new features yet (cursor movement/history come in Step 12 on this same module).
- **Tests:** all existing r11 cases unchanged (prompt editing, Backspace, `cat` reading stdin, Ctrl+D EOF, scrolling).

**As built.** `keyboard/line_discipline.rs`: `LineDiscipline` (the `LineBuffer`, the input row, the prefix and a `Mode`), one
static `LINE_DISCIPLINE`, and three methods -- `begin(console, prefix, mode)` (a fresh row if the cursor is mid-line, else the
cursor's own), `redraw(console)`, and `handle(token, console) -> LineOutcome` (`Ignored`, `Edited`, `Finished(text)`,
`EndOfFile`). Finishing a line -- the UART transcript and the newline on the console -- happens inside `handle`; the caller
flushes the display. `Mode::Prompt` ignores Ctrl+D; `Mode::Canonical` (`read(0)`) turns it into end-of-file on an empty line.
`line.rs` keeps only `LineBuffer` (the `LINE` and `INPUT_ROW` statics are gone); `stdin::read_line` shrank to the polling loop
and the outcome match; `shell::handle_keyboard_irq` lost its drawing and resync code, and `shell::start_prompt` (also used at
boot) does `begin` + `redraw` + the UART prompt. The module doc records the rules the next stages depend on: it knows nothing
about where tokens come from, is never called from interrupt context (so needs no lock once Step 5 lets IRQs through), never
sees signal keys (Stage 20's producer takes those out before queueing), is one instance for the one console, and is a
keyboard-side module that draws on the console. `Console::cursor()` is what it reads to find the row.
Tests: `cases/step04_line_discipline.py` compares one scripted session (prompt Backspace, Backspace on an empty line, a blank
line, Ctrl+D at the prompt, a line wider than the row, an unknown program, exit status, `cat`/`wc` reading stdin with
Backspace and a non-empty Ctrl+D) with `golden/step04_r11.json`, captured from r11 by `mkgolden.py` and checked in so `just test`
doesn't need the r11 directory; all twelve transcripts are identical. It also checks that after output longer than the
screen the prompt is on the last row with output right above it. The one behavior that differs from r11 is the flush: `read_line`
flushed the display for every event; it still does (a program reading a line is not a batch), while the shell flushes once per
batch as before -- so the flush counts are unchanged. Naming: `discipline` is spelled out throughout (a `disc` reads as a floppy).

### Step 5: eval loop out of IRQ context (`main.rs`, `stdin.rs`, `process.rs`, `input.rs`)
Today `handle_keyboard_irq` -> `launch` -> `run_program`, so the GIC interrupt stays unacknowledged for a program's
whole life; that is why every DAIF bit is masked at EL0 and `read(0)` drains the device itself (and why Stage 13
has a "first thing to unmask" hazard). This step removes that structure.
- **Token queue:** fixed-capacity ring buffer of `Token`s (drop-newest on overflow, note on UART; never block in IRQ).
  One producer function `drain_keyboard()` (ack device, `poll`, `input::token_for`, push) callable from the IRQ
  handler *and* from a blocked `read(0)` wait loop (which runs with IRQs masked inside a syscall).
- **IRQ handler** only calls `drain_keyboard()` and EOIs; no drawing, no launching.
- **`kernel_main` ends in the read-eval loop** (never returns -- the "init" role in Stage 12's goal): pop tokens
  (mask IRQs around the empty-check, then `wfi`, to avoid a lost wakeup), feed the line discipline (Step 4), `launch`
  on Enter, redraw the prompt.
- **`read(0)`** pops the same queue through the same line discipline (no separate device-draining path).
- **`run_program`** stops masking DAIF for EL0 (SPSR DAIF clear): keyboard IRQs now just enqueue, so keystrokes typed
  while a program isn't reading are kept, and the reentrancy hazard no longer exists. Re-verify the block-device
  IRQ path while a program runs (blk wait is `wfe`+`peek_used`; `irq_handler` acks it).
- **Docs:** rewrite Stage 13's DAIF-unmask paragraph (raw mode becomes a routing switch on this queue) and any
  Stage 24 notes that assumed the mask; update `process.rs`/`stdin.rs` doc comments.
- **Tests:** everything from before; keys typed while a non-reading program runs (`hello`/a slow program) appear at the
  next prompt in order; typing during `cat` (reads) still works; rapid multi-line input; no lost keys across a
  `cp` of a large file (many blk IRQs during typing).

## Phase 2: the shell

### Step 6: working directory and the shell-state frame stack (`shell_state.rs`, `files.rs`, `fd.rs`)
- `struct ShellFrame { cwd: String, stdio: [StdioBinding; 3] }`, `enum StdioBinding { Default, File(handle) }`;
  `static mut FRAMES: Vec<ShellFrame>` never empty; `push_copy()`, `pop()`, `top()`, `top_mut()`. A comment marks
  where Stage 16 adds `env`. **Two distinct scoping operations (not one):**
  - `with_scope(|| ..)` = `push_copy`/`pop` of the *whole* frame (cwd + stdio, later env): isolates everything, used for
    `./script.sh` / `sh script.sh`.
  - `with_stdio(overrides, || ..)` = save the current frame's `stdio` triple, apply the redirections, run, restore --
    *only* stdio, on the current frame, so state changes made inside (`cd`, a sourced script's `cd`) persist. Used for
    every redirect (Step 8), including on builtins. (This is the "stack of stdio triples" of the original design note.)
  - Builtin/shell diagnostics go through `shell_err(msg)`, which writes via the top frame's `stdio[2]` binding
    (default = console + UART mirror, exactly what `report` does today), so they're redirectable like any program's stderr.
- **Path handling in two functions** (this removes Step 1's root-relative quirk -- see its as-built notes), used by
  `open`, `chmod`, `launch` and `cd`:
  - `abspath(cwd, path)` in `fs/path.rs` (P): pure and lexical, no filesystem access. Joins a relative path onto
    the top frame's cwd, drops `.` and empty components, applies `..` (`..` at the root stays `/`) and enforces the
    limits (a component over 255 bytes or a path over 4096 -> `ENAMETOOLONG`). Named after Python's
    `os.path.abspath`, not `canonicalize`/`realpath`, which in POSIX and Rust also require the path to exist and
    resolve symlinks. Host-tested.
  - `resolve(components)` in `fs/files.rs` (already there since Step 1b): walks a slice of directory components on the
    volume and returns the directory (`ENOENT`, `ENOTDIR`, `EIO`).

  Callers do `resolve(parents)` on the components of `abspath(cwd, path)`. `launch` keeps its bin/ search order
  (bare name, then `name.exe`) but goes through the same functions (no more hand-rolled `find_entry` + `expect`).
- `fd::reset_for_launch()` fills slots 0-2 from the top frame's `stdio`. **Handle ownership:** a `File(handle)` binding
  is a *non-owning reference*; the `with_stdio` guard that opened the file owns it and closes it when it restores (so
  `with_scope`'s copied triple, and `Dup` copies, never own anything, and `pop()` closes nothing). Shell-owned handles
  are marked so `files::close_all()` (run by every launch) skips them; `MAX_OPEN` is sized so a few shell-owned handles
  plus a program's own always fit.
- **POSIX names throughout** (Cygwin is the reference): the shell builtin is `cd`; the kernel-side operation it calls is
  `shell_state::chdir(path)` (validates a directory, sets `top_mut().cwd`; error via `shell_err`); the syscall is `getcwd`
  (Linux aarch64 17, our own arg convention: buf ptr/len, returns length or a negative errno) with `userlib::getcwd`;
  `pwd` is a program (`user/progs/src/bin/pwd.rs`, row in `docs/progs.md`).
- **`cd` details (POSIX subset):** `cd DIR`; `cd` with no operand goes to `/` (POSIX says `$HOME`; no environment
  until Stage 16, which then switches this to `$HOME`); more than one operand -> `cd: too many arguments`; `cd -` and
  `-L`/`-P` unsupported (no `$OLDPWD`, no symlinks) with a clear error; `pwd -L/-P` likewise. `pwd` (program) prints the
  cwd from `getcwd`.
- **No `chdir` syscall yet, deliberately:** with one global frame stack, a child calling `chdir` would change the
  shell's cwd (real Unix's per-process isolation doesn't exist until Stage 19+). Its number (Linux 49) is reserved in
  `abi` with a comment, and the ROADMAP prerequisite table lists it as arriving with per-process state.
- **Tests:** `cd /bin`+`pwd`; relative `ls`/`cat` after `cd`; `cd ..` at root; `cd` to a file / missing dir errors and
  leaves cwd unchanged; `cd` with `.`/`..` mixes; `chmod` and `open` honor cwd.

### Step 7: command-line lexer and `run_line` (`argv.rs` -> `lexer.rs`/`shell.rs`)
- `shlex::split` loses quoting info (`echo "|"` would look like a pipe, `a>b` stays one word). Replace with a small
  lexer producing `Word{text, quoted}`, `Pipe`, and redirection tokens `Redir { fd, op }` with `op` = `In` (`<`),
  `Out` (`>`), `Append` (`>>`), `Dup` (`>&N`), keeping the existing quote/backslash rules for words (unterminated
  quote still `Malformed`). **fd-number prefix (POSIX rule):** an unquoted, unescaped word consisting only of the digit
  `1` or `2` (or `0` before `<`) *immediately* followed by a redirection operator is the fd number (`2>x`, `2>>x`,
  `2>&1`); anywhere else digits are ordinary text (`a2>x` is the word `a2` then `>x`; `echo 2 >x` echoes `2`; a quoted
  `"2">x` is a word). No fd numbers above 2.
- Parser -> `Pipeline { stages: Vec<Command { argv, redirs: Vec<Redir> }> }` -- redirections kept as an ordered list per
  command (order matters for `Dup`, applied left to right like POSIX); syntax errors reported (`>` with no target,
  `2>&` with anything but `1`/`2`, empty pipeline stage, ...). No `&&`/`;` (documented as unsupported).
- **Quoting and comments follow POSIX:** single quotes are fully literal; double quotes keep everything literal except
  that backslash escapes only `"` and `\` (and `$`/backtick, which have no meaning yet -- `\$` inside double quotes yields
  `$`, so this stays compatible when Stage 16 adds `$VAR`); unquoted backslash escapes the next character; a `#` starts a
  comment **only at the start of a word** (`echo a#b` prints `a#b`, `echo a #b` prints `a`). `$`, backtick, `*`, `?`,
  `~` and `{}` are ordinary characters for now (no expansion/globbing until later stages; documented).
- **Messages (bash wording; decided), applied here after the Step 4-5 golden comparisons:** unknown command ->
  `name: command not found` (was `name: not found`); missing exec bit -> `name: Permission denied` (was `not
  executable`); malformed ELF -> `name: cannot execute: Exec format error`. `docs/progs.md` and the tests are updated.
  The `exit N` line stays (our stand-in for `$?`, which needs variables), printed for the *pipeline's* status only when
  nonzero; a documented deviation from POSIX shells, which print nothing.
- Extract the old `Finished` branch into `shell::run_line(&str)`: parse, dispatch builtins (`cd`, later `source`/`sh`),
  else run the pipeline. Plain single commands behave exactly as before.
- **Tests:** quoting cases (`echo "a b"`, `echo '|'`, `echo a\ b`), `a>b` splitting, malformed input reports error and
  prompt survives, every r11 command still works via the new path.

### Step 8: redirection (`shell.rs`, `fd.rs`, `files.rs`)
- `cmd > f`: shell opens `f` for write (via `abspath` and `resolve`; creates/truncates, same limits as `open`),
  `with_stdio`: bind stdio[1] to the handle, run, restore (not `push_copy`, which would discard state changes a
  redirected builtin makes). `cmd < f` symmetric on stdio[0] (reader). `cmd > f < g`
  both. Failure to open (missing input, read-only output, directory) reports an error and doesn't launch.
- **stderr redirection (decided: included).** The frame's `stdio` triple already has a slot for fd 2, so `cmd 2> f`
  (truncate) and `cmd 2>> f` (append) are the same code path as stdout with `stdio[2]` as the target -- nothing new
  in the kernel or syscalls. `cmd 2>&1` / `cmd >&2` (`Dup`) copy one slot's binding into another: the copy is a
  non-owning reference to the same shell-owned handle (same underlying writer, so interleaved output keeps
  order and one shared position), closed once by the `with_stdio` that opened it. Redirections apply strictly left to right, so
  `cmd > f 2>&1` sends both to `f` while `cmd 2>&1 > f` sends stderr to the *old* stdout (the console) and only
  stdout to `f`. `2>&1` on a stdout that is the console is a harmless no-op. (`2>&1` is included because it needs
  nothing beyond `2>`; drop it if you'd rather keep Step 8 to the listed forms.)
- **`cmd >> f` (decided: supported).** `hadris-fat` 2.4.0 already has `FileWriter::new_append` (positions at the
  end of the chain, `finish()` updates the size), so this is small: `abi::fs::O_APPEND` (Linux value `0o2000`), combined with
  `O_WRONLY`, creates the file if missing and does *not* truncate; `files::open_write(path, append)` picks
  `new_append` vs the truncating writer. `cp` is unchanged. Also lifts Stage 11's "no append" restriction for programs
  (`docs/progs.md` and `files.rs` comments updated); a scope-wide `./s.sh >> out` uses one shell-owned append handle.
- fd numbers above 2 (`3>`) and here-documents are unsupported.
- **Redirection applies to builtins, POSIX-style (decided).** `cd > f` creates/truncates an empty `f` and still changes
  directory (`with_stdio` keeps the cwd change); `cd nosuch 2> e` captures the error in `e`; `source s.sh > out`
  redirects everything the sourced script prints while its own `cd`s persist; `sh s.sh > out` / `./s.sh > out` redirect a
  scoped script's output. The redirect files are opened before the command runs and closed after; a failed open reports
  and skips the command (builtin or not). Like POSIX shells, earlier redirects of the same command line have already
  taken effect if a later one fails (`cmd > f < missing` leaves `f` truncated). Launch/lookup errors (`command not found`,
  `Permission denied`, `Exec format error`) are emitted *inside* the `with_stdio` scope, so `nosuch 2> e` captures the
  message in `e`, as bash does.
- **Tests:** `ls > listing.txt` then `cat listing.txt` (and mtools check on the host image); `cat < file`; in and out
  together; `>` to an existing file replaces; `>` to a read-only file errors; `<` of missing file errors; `false > f`
  still reports `exit 1`; stdout redirect doesn't hide stderr.

### Step 9: scripts and scopes (`shell.rs`)
- `run_script(path, scoped)`: read the file, feed each line through `run_line` (recursion in the same interpreter,
  not a new process). `scoped` => `with_scope` (push_copy/pop) so `cd` and any redirect held by the script don't leak;
  unscoped runs against the current frame. A redirect on the invoking line (`./s.sh > out`, `source s.sh > out`) is
  a Step 8 `with_stdio` wrapped around the whole script run, so it covers every line the script executes.
- Invocation: `source FILE` / `. FILE` (`.` is the POSIX name, `source` the bash/Cygwin one) unscoped; `sh FILE` and
  `./FILE` (any word containing `/` resolving to a file with the exec bit) scoped -- POSIX runs those in a new shell
  process, which is exactly what a pushed frame stands in for. **bash's `ENOEXEC` fallback:** an exec-bit file without ELF
  magic whose first 128 bytes contain no NUL is run as a script; otherwise (binary garbage) it's `cannot execute binary
  file: Exec format error`. (`sh FILE` doesn't need the exec bit, like POSIX; `source FILE` neither.) `#!` lines are just
  comments (there is only one interpreter). Extra arguments to a script (`sh f a b`) are rejected with an error -- there
  are no positional parameters until variables exist. Depth cap (16) to protect the 1 MiB kernel stack. Blank lines and
  `#` comments skipped; a failing line reports and the script continues (POSIX default without `set -e`). No `exit`
  builtin (documented: a script runs to its last line); no `if`/`for`/functions.
- **Tests (fixtures on the image; exec bit via `chmod +x` in the test):** `./cdbin.sh` (contains `cd /bin`) leaves
  `pwd` unchanged; `source cdbin.sh` moves it; nested scripts (inner `cd` doesn't leak outward; outer's doesn't leak
  past its pop); recursion depth cap error; script with a redirect `./s.sh > out` (scope-wide, shell-owned handle,
  file contains all lines' output) leaves the redirect gone after pop; missing/non-exec script errors.

### Step 10: `mkdir`, `rm`, `mv` (`files.rs`, `syscall.rs`, `abi`, `userlib`, `progs`)
- Syscalls over `hadris-fat`'s `create_dir`/`delete`/`rename` (first confirm exact APIs and their limits, e.g. delete of
  non-empty dirs, rename across directories, long-name handling). Linux-style numbers where they exist
  (`mkdirat`/`unlinkat`/`renameat` shapes simplified to path ptr/len args).
- `hadris-fat`'s `delete` removes a file or an **empty** directory only (it scans for anything besides `.`/`..`), so
  recursion is ours. Kernel syscalls: `mkdir(path)`, `unlinkat`-shaped `unlink(path, flags)` (flag `AT_REMOVEDIR`
  = `0x200` for a directory, empty only), `rename(old, new)`.
- Programs (POSIX subsets; all continue past a failing operand and exit 1 if any failed, like POSIX): `mkdir DIR...`
  (no `-p`/`-m`; parent must exist, existing -> `File exists`); `rm [-r] PATH...`: plain `rm` refuses a directory
  (`Is a directory`, POSIX-like), refuses an operand whose last component is `.` or `..` and, as a safety guard, the root
  `/`, and refuses a read-only file with `Permission denied` (POSIX `rm` would prompt/`-f`; FAT's read-only bit is the
  closest analogue); **`-r` (decided) drills into directories** and deletes children before the directory. Implemented in the program with no
  fd held across recursion (the fd table has 16 slots and `files.rs` only 8 open files): loop { open the directory,
  read one record with `getdents`, close, delete it -- recursing first if it's a directory -- }, until the
  directory is empty, then remove it; depth bounded by FAT's path-length limit, and the EL0 stack from Step 3 is
  generous enough. `-f` and a separate `rmdir` program are not included (not requested; noted in `docs/progs.md`).
  `mv SRC DST` follows POSIX `rename` semantics for two operands: if `DST` is an existing directory the source moves
  *into* it (`DST/basename(SRC)`); if `DST` is an existing file it is replaced (if `hadris-fat`'s `rename` won't replace,
  `mv` removes the target first only when that can't lose data, else fails with `File exists` -- a documented deviation);
  a directory can't be moved into itself or a descendant (`Invalid argument`); `mv a a` -> error; a trailing `/` on `DST`
  requires an existing directory; more than two operands, `-f`, `-i` unsupported; attributes (read-only, exec bit) move
  with the entry.
  Each gets a `docs/progs.md` row with the supported/unsupported subset.
- **Tests:** create/list/remove; error cases (exists, missing parent, non-empty, read-only, exec-bit preserved by
  `mv`); host-side mtools check of resulting image.

### Step 11: pipes via temp files (`shell.rs`)
- `a | b [| c ...]`: run stage 1 with stdout -> temp file, next stage with stdin <- that file, delete afterward;
  multi-stage chains use a fresh temp per link (unique names from a counter, so a script running its own pipeline
  inside a pipeline stage can't collide); temp files live in `/tmp/` on the image (created by `just disk`) via absolute
  paths so they're cwd-independent; removed on every exit path. Named limitation (unchanged from the roadmap): finite
  output that fits on disk only.
- **POSIX pipeline semantics, as far as sequential execution allows:** every stage always runs, even if an earlier one
  failed, faulted or wasn't found (a stage that can't start acts as an empty producer/consumer and reports its error);
  the pipeline's status is the *last* stage's; the pipe is bound *before* the stage's own redirections, so `a > f | b`
  sends `a`'s output to `f` (b sees empty input) and `cmd 2>&1 | b` sends stderr through the pipe too. Only failing to
  create/write the temp file (disk full, `/tmp` missing) aborts the pipeline. The first stage's stdin is the keyboard.
  Deviation: stages run one after another, not concurrently, so e.g. `yes | head` can't work (Stage 23).
- `> `/`<` on the first/last stage combine with pipes (`a < in | b > out`).
- **Decided: temp file** (the ROADMAP's deliberate MS-DOS-style design; exercises the filesystem and stays bounded by
  disk, not RAM). The kernel-heap `Pipe(Vec<u8>)` alternative is not built; it is recorded as a possible later option in
  the ROADMAP (still finite, streaming pipes need Stages 19/23). The heap growth in Step 1 is independent of this.
- **Tests:** `echo hello | cat`; `ls | wc -l`; 3-stage chain; `cat file | head -n 3`; a stage that fails or faults still
  cleans up its temp file and reports; pipe + redirect combos; no leftover files in `tmp/` (host check).

### Step 12: line editing and history (`keyboard/line.rs`/`line_discipline.rs`, `console/mod.rs`)
- Cursor-aware buffer (Stage 5's insert/remove at a position, adapted -- no CSI parsing, no ANSI redraw), driven by
  discrete keys: Left/Right/Home/End/Delete/Backspace and the readline basics Ctrl+A (start), Ctrl+E (end), Ctrl+U
  (kill to start), Ctrl+K (kill to end) -- exactly these; Ctrl+W/L/R, Tab completion, `!!`, and a history file are out of
  scope. Ctrl+D at the prompt does nothing (POSIX interactive shells exit on EOF; this one is the init process and never exits --
  documented). Redraw by rewriting the changed
  cells via `put_char`; block-glyph cursor drawn as an ordinary glyph (virtio-gpu has no text cursor);
  works with `show_row`'s sliding-window logic for lines wider than the row.
- History: ring buffer of past lines (skip empties/duplicates of the last), Up/Down replace the current line
  (keeping the in-progress line to restore at the bottom).
- Editor lives in the Step 4 line-discipline module in **two modes**. *Prompt mode* = everything above. *Canonical mode*
  (`read(0)` for programs) follows a real tty's canonical discipline, not readline: Backspace (ERASE), Ctrl+U (KILL: discard
  the line), Ctrl+D (EOF), no cursor movement, no history (a real cooked-mode tty doesn't interpret arrow keys either).
  Refinement over r11, POSIX-correct: Ctrl+D on a *non-empty* line delivers the pending bytes without a newline (the
  next `read` sees them; a further Ctrl+D on the now-empty line is EOF); r11 ignored it. (Made after the golden comparisons.)
- **Tests (via `sendkey` incl. arrow/home/end/delete):** insert mid-line, delete, Home/End, Up/Down through several
  commands and back to the pending line, long-line scrolling window, Backspace at column 0, history doesn't record
  `read(0)` input, `cat` stdin editing.

### Step 13: docs, roadmap, full regression
- `Stage12.md` and the ROADMAP.md Stage 12 summary: reconcile with what was actually built (Steps as implemented,
  the frame stack = cwd + stdio triple with `env` reserved for Stage 16, pipes decision, limitations list, any
  decisions that changed); Stage 13 (raw mode is a routing switch on the token queue), Stage 16 (adds `env`
  to the existing frame; helper/`x2`/entry macro; demos 2 and 3 unchanged), Stage 24 (drop mask assumptions).
- `docs/progs.md`: `pwd`, `mkdir`, `rm`, `mv` rows + a shell section (grammar, builtins, scoping rules, limits).
  A `docs/shell.md` if the section outgrows progs.md.
- **"Path to a userspace shell" section in ROADMAP.md** (decision: the shell stays kernel-resident in Stage 12, as the
  roadmap already says, but the plan is forward-aware and the roadmap records how later stages complete the move).
  Written into Stage 12 as a short subsection and cross-referenced from each stage it names (Stages 16-19, 22-23):

  | Userspace-`sh` prerequisite | What provides it | Stage-12 preparation |
  |---|---|---|
  | Shell is not in IRQ context; `kernel_main` is an init loop that starts/restarts it; one input queue independent of reader | Step 5 (token queue + eval loop) | done here |
  | Per-process state (`cwd`, stdio bindings, later `env`) that a child inherits | Stage 16 adds `env`; Stage 19's slots hold a process struct | `ShellFrame` is plain data (no globals inside its API), so it becomes the per-process struct |
  | Two programs resident at once (shell stays loaded while a child runs) | Stage 19 (second window, per-slot `KERNEL_CTX`) with 17/18 for sizing/heap | none; noted only |
  | `spawn`/`wait` syscalls (fork/exec equivalent) | Stage 19/22 (a resident program starts another and collects its status) | `run_program`'s launch path split into "load+setup" and "run" so it can be reused |
  | `chdir` syscall (POSIX name; the shell builtin stays `cd`), `dup2`-style fd control for redirection | new syscalls once state is per-process; `getcwd` already added in Step 6, `chdir` number reserved in `abi` | fd binding/redirect logic kept in one module (`shell_state`/`fd`) |
  | Real pipes between resident programs, job control | Stages 23 and 20-22 | pipe/temp-file code isolated in one function so it's replaceable |
  | Script scoping (`./s.sh` vs `source`) becoming a real child process vs frame push | Stage 19+ spawn | Step 9's `with_scope` is the same operation the child process would perform |

  Edits: Stage 12 (new subsection + kernel-resident rationale), Stage 16 (env joins the frame; "per-process struct"
  note), Stage 19 (states it is the gate for a userspace shell and what that would require), Stage 22/23 (job
  control/pipes reference the same path). Stage 9-11 text untouched.
- Full `just test` for r12; rebuild/run r09-r11 demos against the modified `user/` crates; final pass on stale comments.

---
## Design constraints from the forward-awareness decision (apply during Steps 5, 6, 9, 11)
- `ShellFrame`/stdio bindings are plain data with no dependence on statics inside their own methods (the *stack* of
  them is the static), so they can later be embedded in a per-process struct unchanged.
- Split `run_program` into `prepare` (load ELF, build stack/argv, fd setup) and `run` (enter EL0/return) in Step 1's
  `push_cstr_array` refactor, so a later spawn syscall reuses `prepare`.
- Keep the token queue reader-agnostic (Step 5): whoever is foreground pops it; nothing assumes the kernel shell is the reader.
- Keep pipe plumbing (Step 11) in one function behind a `run_pipeline` interface.

## Testing (authoritative; the short "Tests" bullets inside each Step summarize this)

### Infrastructure (built in Step 0, extended as needed)
- **Two layers.** (1) `just test-host`: pure logic tested on the host with plain `cargo test` -- possible because the
  logic modules are written `no_std` + `alloc` with no kernel dependencies and are pulled into a tiny host crate
  (`r12_shell/hosttests/`, `#[path = "../../src/<mod>.rs"]`): `path.rs` (`abspath`), `lexer.rs`/parser, `editor.rs`
  (cursor-aware line buffer, no console), `history.rs`, `utf8.rs` (streaming decoder), the token queue ring, and
  `push_cstr_array`'s layout math. (2) `just test-qemu`: headless QEMU driven by the sendkey harness. `just test` runs both.
- **Harness additions** (`test/run_tests.py`, from r11's `Session`): fail immediately if the serial log ever shows
  `Kernel Panic!` or `Unexpected exception`, or QEMU exits early; sparse copy of the image; `KEY_NAMES` extended for
  `> < | " ' \ # ~ ;` (`shift-dot`, `shift-comma`, `shift-backslash`, `shift-apostrophe`, `apostrophe`, `backslash`,
  `shift-3`, `shift-grave_accent`, `semicolon`) and named keys (`up down left right home end delete ctrl-d`);
  `Session.run(cmd)` returns the transcript for one command; `Session.screendump()` via the monitor's `screendump`
  (PPM parsed in Python) for the few checks that need the display, not just the serial mirror.
- **Host-side image checks** after QEMU quits: `mtype`/`mdir`/`mcopy` to read files back, and `fsck.fat -n` must be clean
  for the final image of every session that wrote (append, `rm -r`, `mv`, pipes) -- catches FS corruption the console can't show.
- **Golden transcripts (Steps 4-5):** one scripted session (prompt editing, `echo`, `cat` on stdin, `ls`, `cp`, errors)
  is run on r11 and on r12 and the normalized serial logs must be identical -- the strongest guard that the
  restructuring didn't change behavior.
- **Placement rule (decided): tests live in this stage's `disk/tests/`; `user/` holds only the common binaries that
  build up the core-utils set.** Nothing test-only goes into `user/progs/`. (Stage 9's `hello`/`crash` already live
  there and stay, since r09/r10 use them.)
- **Test programs** are a separate Cargo package, `r12_shell/test/progs/` (own `Cargo.toml`, `.cargo/config.toml`,
  `link.ld` copy -- every user binary links at `0x44000000` -- depending on `userlib` and the `progs` lib helpers by path).
  `just disk` builds it and stages each binary as `disk/tests/<name>.exe` (generated files gitignored: add
  `**/disk/tests/*.exe` next to the existing `**/disk/bin/*.exe` rule). Programs: `probe` (syscall probing: unknown syscall,
  bad pointers, fd exhaustion, argv layout/alignment, `getcwd`, deep stack use), `overflow` (unbounded recursion),
  `spin N` (busy-wait ~N seconds; no sleep syscall exists yet -- time-based via `CNTVCT_EL0` if EL0 access is enabled
  (set `CNTKCTL_EL1.EL0VCTEN` in Step 5 if needed), otherwise a calibrated iteration count; the tests only need "long
  enough to type during", not exactness). Documented in `r12_shell/test/README.md`,
  not `docs/progs.md`. The `bigpad` fixture is host-made at image-build time (`hello.exe` + 3 MiB of trailing zeros,
  valid because the loader ignores bytes past the segments) into `disk/tests/`.
- **Exec bit and invocation:** the kernel's boot-time exec-bit marking covers only `bin/`, and `launch` finds bare
  names only in `bin/`. Tests therefore run test programs by path (`/tests/probe.exe`; after Step 6 also relative, e.g.
  `cd /tests` then `./probe.exe`) and the harness starts each session with `chmod +x /tests/<name>.exe` for the ones it
  needs (which also exercises `chmod`). This needs "a word containing `/` runs that path" in `launch` from Step 1 on.
  In the per-step lists below, bare names like `probe`, `overflow`, `spin`, `cdbin.sh`, `binary256`, `notes.txt` all mean
  files under `/tests/`.
- **Fixtures** are static files checked in under `r12_shell/disk/tests/`: text (`notes.txt`), binary (`binary256`, all
  256 byte values), multibyte UTF-8, a file >4096 bytes with a multibyte character straddling offset 4096, a
  read-only file, a 3-level directory tree for `rm -r`, and scripts (`cdbin.sh`, `nested.sh`, `outer.sh`, `inner.sh`,
  `redir.sh`, `bad.sh`). The pipe temp directory `disk/tmp/` is part of the image layout, not a test file. Malformed ELFs
  (truncated, segment out of window, bad `e_phoff`) are generated by `just disk` into `disk/tests/` (gitignored, like
  the `.exe` files) so no binary blobs are checked in.

### Per-step tests (expected results are exact strings/behaviors unless noted)
**Step 0.** T0.1 every r11 utility case passes on r12 as `cases/core_utils.py` (fixture paths under `tests/`; the root listing is `bin fonts tests tmp`). T0.2 the built image is 64 MiB FAT16, `fsck.fat -n` clean.
T0.3 `git status` shows nothing under `rust/r09..r11`. T0.4 `just test-host` runs (zero tests is acceptable here).

**Step 1.** T1.1 `chmod +x /tests/binary256` then `/tests/binary256` -> `...: cannot execute: Exec format error`, prompt returns,
`echo ok` works (a text file gets the same message until Step 9 adds the script fallback; `/tests/notes.txt` likewise checked here).
T1.2 truncated ELF (100 bytes of `echo.exe`, exec bit set) -> same message, no panic. T1.3 ELF whose segment lies outside
the window -> same. T1.4 ELF with `e_phoff` past EOF -> same. T1.5 `bigpad` (3 MiB file) runs and prints its output
(needs the 16 MiB heap; would have panicked at 1 MiB). T1.6 `probe`: unknown syscall returns `-38`; `write` with a
bad pointer returns `-14`; `open` with a bad pointer `-14`; opening files until failure fails with `-24` at exactly
the documented limit (`MAX_FDS - 3` = 13, `MAX_OPEN` made equal). T1.7 argv: `probe args a "" b` prints argc=4,
argv[2] empty, every `argv[i]` 8-byte aligned pointer, initial `sp` 16-byte aligned; 30 arguments work. T1.8 host: `abi`
constants equal the values r11's `syscall.rs`/`errno.rs` use (so r09-r11 binaries still work). T1.9 r11 `just test`
still passes against the modified `user/` crates.

**Step 2.** T2.1 `cat binary256` -> serial transcript contains all 256 byte values in order (after `\n`->`\r\n`).
T2.2 `cat` of the >4096-byte UTF-8 fixture with a multibyte char at offset 4096: screendump row crops equal the same
text typed as a single write (no mangled glyph at the boundary). T2.3 `echo é` and `cat` of a file containing byte
0x82 render the *same* glyph (screendump cell equality; only until Step 2b, where a lone 0x82 becomes U+FFFD -- see T2b).
T2.4 host: `utf8.rs` decoder -- valid 1-4 byte sequences, split at every possible boundary, overlong/invalid/truncated
sequences (invalid byte -> U+FFFD, decoder resyncs).
T2.5 `probe` prints 200 `write!` fragments in one line: GPU flush count (kernel debug counter exposed via a UART
line at exit) is bounded (one per syscall, and `Fd` sends far fewer syscalls than fragments). T2.6 `probe` writes `OUT`
(no newline) to stdout, `ERR` to stderr, then `\n`: under `> f 2>&1` the file is `OUTERR\n` (needs Step 8's redirection,
so it is checked there; until then the console shows the same order).

**Step 2b.** T2b.1 host (`std` allowed, includes the crate): `glyph_for` classification -- ASCII width 1, CJK width 2,
zero-width set draws nothing, character above U+FFFF -> U+FFFD, uninterpreted control -> U+FFFD. T2b.2 host: the
cell-grid rules on a pure grid model -- wide glyph = `WideLeft`+`WideRight`; wrap at the last column; overwriting half of
a wide glyph blanks the other half; scrolling moves the grid with the rows; BS after a wide glyph moves back two cells,
after a narrow one cell, at column 0 stays; BS twice over `a` + wide + `b`. T2b.3 host: `show_row`'s window in cells --
a line of wide characters wider than the row keeps a whole-character window that never starts on a `WideRight`.
T2b.4 CJK fixture: screendump shows two cells per glyph, serial transcript is the raw UTF-8. T2b.5 a program writing
`<wide>\b` then a marker: the marker lands on the wide glyph's left cell. T2b.6 `echo é` draws Unifont's `é`; a lone
byte 0x82 and an emoji draw U+FFFD. T2b.7 `cat binary256` draws a run of U+FFFD, no panic, serial transcript still
byte-exact. T2b.8 the ASCII baseline is re-captured after this Step (Unifont's ASCII differs from r11's) and later
Steps compare against it.

**Step 3.** T3.1 `overflow` -> serial `Segmentation fault ...`, prompt shows `exit 139`, next command works, and
`hello` afterward prints normally (no corruption). T3.2 `probe stack 256` (uses 256 KiB of stack legitimately) succeeds
(guards the stack size floor). T3.3 `tail` with its 512 KiB static buffer and every r11 program still run. T3.4 a
program that reads just past the top of its stack region faults (guard is real). T3.5 (verification result recorded in
the docs): where the stack pages are mapped, and that nothing depends on the previous program's mapping.

**Step 4.** T4.1 typing/Backspace/Enter at the prompt; T4.2 Backspace on an empty line is a no-op; T4.3 `cat` reading
stdin: two lines echoed then Ctrl+D EOF; T4.4 Ctrl+D on a non-empty line does nothing; T4.5 line wider than the row
(sliding window) still runs correctly; T4.6 more output than screen rows scrolls and the prompt lands on the last row
(screendump: prompt glyphs present at the bottom row); T4.7 golden-transcript equality with r11.

**Step 5.** T5.1 `spin 3` then type `echo a`,`echo b` during it: after it ends they run in order, once each. T5.2 keys
typed while `cat` (a reader) runs are consumed by `cat`, not the shell. T5.3 with a queue capacity of 256 tokens:
200 `sendkey`s during `spin` are all processed in order; 600 during `spin` produce the documented overflow note on the UART
(newest dropped, first 256 kept) and the system stays responsive (`echo alive` works). T5.4 `cp bigfile copy` (many blk IRQs) while typing: file identical on the host (`cmp`), typed
line intact. T5.5 30 rapid short commands in a row: outputs in order, no re-entrancy symptoms, no panic. T5.6 golden
transcript equality with r11. T5.7 host: token ring (FIFO order, wraparound, overflow drop-newest, empty check).

**Step 6.** T6.1 `pwd` -> `/`; `cd bin`, `pwd` -> `/bin`; `cd ..`, `pwd` -> `/`; `cd ..` at root stays `/`;
`cd /bin/../fonts` -> `/fonts`; `cd ./bin/.` -> `/bin`. T6.2 `cd nosuch` -> error, cwd unchanged; `cd notes.txt` (a
file) -> `Not a directory`, cwd unchanged. T6.3 after `cd bin`, `ls` lists bin; `cat ../notes.txt` works;
`chmod -x cat.exe`/`+x` operate relative; open from a program is cwd-relative. T6.4 `probe getcwd` with a too-small
buffer returns an error (no overflow). T6.5 host: `abspath` table (absolute/relative, `.`/`..`, repeated `/`,
trailing `/`, `..` past root, empty string). T6.6 host: frame stack (push_copy/pop restore cwd and stdio; pop of the
base frame is refused; shell-owned handles listed for closing); `with_stdio` restores only stdio and a cwd change made
inside it persists; nested `with_stdio` and `with_scope` interleavings restore correctly; early-return paths unbalance nothing.

**Step 7.** T7.1 host lexer/parser table: `echo a b` -> [echo,a,b]; `echo "a b"` -> one word; `echo 'a|b'` -> one
quoted word (not Pipe); `echo a\ b`; `a>b` -> [a, >, b]; `a | b > c < d`; `>` with no target and `|` at either end ->
syntax error; fd prefixes: `cmd 2>x` -> Redir{2,Out}; `cmd 2>>x`; `cmd 2>&1` -> Redir{2,Dup(1)}; `cmd >&2`;
`a2>x` -> word `a2` + `>x`; `echo 2 >x` -> args [2] + `>x`; `echo "2">x` -> quoted word `2` + `>x`; `3>x` -> word `3`
+ `>x`; `2>&` / `2>&x` -> syntax error; redirect order preserved in the list; unterminated quote -> Malformed; empty/whitespace line -> Empty; `#` comment lines. T7.2 QEMU: `echo "|"`
prints `|`; `echo a>b` is the redirect form (after Step 8); a malformed line reports an error and the prompt survives;
every r11 command line behaves the same through the new path.

**Step 8.** T8.1 `ls > listing.txt`; `cat listing.txt` matches `ls`; host `mtype` equals it. T8.2 `echo x > f` then `echo y > f`
-> file is `y` (truncate); `echo z >> f` -> `y` then `z`; `>>` on a missing file creates it. T8.3 `cat < listing.txt`
prints it; `cat < listing.txt > copy.txt` (both) and `cmp`. T8.4 error cases: `< missing` -> error, command not run;
`> readonlyfile` -> `Permission denied`; `> bin` (directory) -> `Is a directory`; `cat >` -> syntax error.
T8.5 `false > f` reports `exit 1`. T8.5a builtins: `cd bin > f` -> `f` exists and is empty AND `pwd` is `/bin`;
`cd nosuch 2> e` -> `e` holds the error text, console silent, cwd unchanged; `cd nosuch > o` -> error still on the console,
`o` empty; `source cdbin.sh > out` -> cwd changed, script output in `out`; `./cdbin.sh > out` -> cwd unchanged, output in `out`;
a failed redirect open on a builtin (`cd bin > missingdir/f`) reports and does not run the builtin (cwd unchanged). T8.5b stderr: `probe out-err` (writes `OUT` to fd 1 and `ERR` to fd 2) -- plain: both on the
console; `> o`: `ERR` on the console, `o` = `OUT`; `2> e`: `OUT` on the console, `e` = `ERR`; `> o 2> e`: files split;
`2>> e` appends across two runs; `> f 2>&1`: `f` holds both (in program order); `2>&1 > f`: `ERR` on the console, `f` = `OUT` only;
`2>&1` with stdout on the console changes nothing; a missing `2> dir/x` target errors and doesn't launch; `2> e < in > o`
combined; host `mtype` verifies file contents; `fsck.fat -n` clean.
T8.6 large redirect (bigfile -> copy) identical on the host. T8.7 `>>` twice interleaved with reads keeps size exact
(FAT size correct via `fsck.fat -n`).

**Step 9.** T9.1 `./cdbin.sh` (`cd /bin`, exec bit via `chmod +x`) then `pwd` -> `/` (scoped); `source cdbin.sh` then
`pwd` -> `/bin`; `. cdbin.sh` same as source; `sh cdbin.sh` scoped. T9.2 nested: `outer.sh` runs `cd /fonts`, `./inner.sh`
(`cd /bin`, `pwd`), `pwd` -> inner prints `/bin`, outer prints `/fonts`, shell afterward `/`. T9.3 depth cap: a
self-recursive script stops at 16 with a clear error, kernel stack intact, prompt returns. T9.4 `./redir.sh > out`
collects every line's output; afterward the shell's stdout is the console again. T9.5 `./bad.sh` (a failing line in
the middle): error reported, remaining lines still run, script returns. T9.6 comments/blank lines skipped; missing script,
non-exec script (`Permission denied`), directory as script -> errors, prompt survives. T9.7 no leaked open handles after
scripts (open 13 files succeeds in `probe`). T9.8 `ENOEXEC` fallback: `chmod +x /tests/notes.txt` then running it executes its
lines as commands (each an unknown command -> `command not found`, script finishes); `chmod +x /tests/binary256` ->
`cannot execute binary file: Exec format error`; a truncated ELF is *not* run as a script (ELF magic wins). T9.9 `sh f a b`
-> error (no positional parameters); `sh f` works without the exec bit; `#!/bin/sh` first line is ignored as a comment.

**Step 10.** T10.1 `mkdir d`, `ls` shows `d`; `mkdir d` again -> exists error; `mkdir nosuch/x` -> `No such file or
directory`. T10.1b `mkdir a b c` creates all three; `mkdir a x` (a exists) reports for `a`, still creates `x`, status 1.
T10.2 `rm f` removes a file; `rm f g` continues after an error on `f`, status 1; `rm .`, `rm ..`, `rm /`, `rm -r /` -> refused; `rm d` (directory) -> `Is a directory`; `rm -r d` removes a populated tree
(3 levels, several files per level, host-built fixture) and leaves siblings alone; `rm missing` -> ENOENT; read-only
file -> `Permission denied`; `rm -r` on a file works like `rm`. T10.3 `mv a b` renames; `mv a d` (d an existing directory) and `mv a d/` move into `d/a`;
`mv a nodir/` -> error; `mv dir dir/sub` -> `Invalid argument`; `mv a a` -> error; `mv a b` where b is an existing file
replaces it (or fails with `File exists` per the documented rule); exec bit and read-only bit preserved; `mv missing x` -> error;
three operands -> usage error. T10.4 after each mutation `fsck.fat -n` clean; long filenames (LFN > 13
chars) survive `mv`/`rm`. T10.5 `rm -r` never holds more than a couple of fds (run with the fd table nearly full).

**Step 11.** T11.1 `echo hello | cat` -> `hello`; `ls | wc -l` matches `ls` line count; 3-stage `cat listing.txt | head -n 3 | wc -l`
-> `3`; `cat bigfile | wc -c` equals the host size (large intermediate). T11.2 `cat < in | wc -w > out` (redirects on ends).
T11.3 failure cases (POSIX: every stage runs): first stage missing program -> `command not found` and the next stage still runs
with empty input (`nosuch | wc -c` prints `0`); middle stage faults (`crash`) -> the following stage still runs on whatever was
written, temps cleaned; a stage exiting nonzero doesn't abort the rest; pipeline status = last stage (`exit N` line
only for the last stage's nonzero status: `false | true` -> none, `true | false` -> `exit 1`). T11.3b `a > f | b`: f gets
`a`'s output, `b` sees empty input; `probe out-err 2>&1 | wc -c` counts both streams; `probe out-err | wc -c` counts only
stdout (stderr on the console). T11.4 disk full during a pipe (pre-fill the image on the host) -> clean error, cleanup,
prompt returns. T11.5 host check: `/tmp` empty after every scenario; `fsck.fat -n` clean. T11.6 `cd /bin` then a pipe: temp
paths are absolute so they're cwd-independent. T11.7 a script containing a pipe.

**Step 12.** T12.1 insert mid-line: type `echo ac`, Left, `b` -> `echo abc`; Home/End/Delete/Backspace positions
exact; Ctrl+A/E/U/K if implemented. T12.2 history: run 3 commands; Up x3 recalls in reverse, Down returns to the pending
(half-typed) line; duplicates of the last command aren't re-recorded; empty lines aren't recorded; history ring wraps at
its cap. T12.3 line wider than the row with the cursor near both ends: visible window follows the cursor, edits land
in the right place (screendump: block cursor on the correct cell). T12.4 canonical mode for programs (`cat` reading stdin): Backspace edits, Ctrl+U discards the line, Left/Right/Up/Down/Home/End do
nothing and nothing enters history; Ctrl+D on `abc` delivers `abc` with no newline, then Ctrl+D again is EOF.
T12.4b prompt mode extras: Ctrl+A/E/U/K exact results; Ctrl+D at the prompt does nothing. T12.5 host: `editor.rs` and `history.rs`
state-machine tables (insert/delete at every position, including wide characters (positions counted in cells, whole-character moves, Step 2b's rules), ring semantics, pending-line restore).
T12.6 manual (`just run`, real display): cursor visibility/blink-free block, feel of redraw, no flicker.

**Step 13.** T13.1 every test above in one clean `just test` from a fresh checkout state (delete `disk.img`, `target/`).
T13.2 r09, r10, r11 build and (r11) pass their tests against the modified `user/` crates. T13.3 docs check script:
every program in `user/progs/src/bin/` has a `docs/progs.md` row; every syscall in `abi` appears in the docs table;
every program in `r12_shell/test/progs/` is described in `r12_shell/test/README.md`; nothing test-only exists under `user/`
(`git diff` of `user/progs/src/bin/` adds only `pwd`, `mkdir`, `rm`, `mv`).
T13.4 the acceptance demo below, run by hand on the display.

## Stage 12 complete: the final state
**Boot and kernel structure.** `kernel_main` initializes memory (16 MiB heap), MMU, GIC, block device, FAT volume,
font, GPU/console, keyboard, then enters the read-eval loop and never returns. Keyboard IRQs only enqueue tokens; the
shell (kernel-resident, per the ROADMAP) consumes them; programs run with IRQs enabled; `read(0)` pops the same queue
through the same line discipline. No shell code runs in IRQ context.
**Kernel modules (new/changed):** `shell.rs` (run_line, builtins, scripts, pipelines), `lexer.rs`, `shell_state.rs`
(frame stack), `path.rs`, `line_discipline.rs` + `editor.rs` + `history.rs`, `tokenq.rs`, `utf8.rs`, `files.rs` (`resolve`, append,
mkdir/unlink/rename), `elf.rs` (fallible), explicit user stack + guard, `abi` crate shared with `user/`.
**Syscalls (all with `abi` constants; Linux aarch64 numbers):** getcwd 17, mkdirat 34, unlinkat 35 (`AT_REMOVEDIR`),
renameat 38, chmod 53, open 56 (+`O_APPEND`), close 57, getdents 61, read 63, write 64, exit 93; `chdir` (49) reserved,
unimplemented by design (see the userspace-`sh` table); unknown -> `ENOSYS`, bad pointer -> `EFAULT`.
**Shell language:** words with `'`/`"`/`\` quoting, `#` comments, `|`, `<`, `>`, `>>`, `2>`, `2>>`, `2>&1`/`>&2`; builtins `cd`, `source`/`.`, `sh`;
`./script` for exec-bit scripts; no variables/`$?`/`;`/`&&`/globbing/background jobs/fds above 2 (documented as not supported);
line editing (arrows/Home/End/Delete/Backspace), 
history; prompt `> `.
**User programs (`user/progs`, core utils only):** echo cat ls cp head tail wc hexdump true false chmod (Stage 9-11) +
`pwd mkdir rm mv` (new), plus Stage 9's `hello`/`crash`; all with rows in `docs/progs.md`. **Test programs and fixtures**
live only in `r12_shell/test/progs/` -> `disk/tests/` (`probe overflow spin` + fixtures), documented in
`r12_shell/test/README.md`; the shared `abi` crate sits beside `userlib` as a library, not a binary.
**Disk and memory:** 64 MiB FAT16 (`bin/ fonts/ tmp/` + fixtures), gitignored image; kernel heap 16 MiB, DMA pool 2 MiB,
kernel stack 1 MiB, user window unchanged at 2 MiB (variable size is Stage 17).
**Docs/repo:** ROADMAP Stage 12 restructured with the Steps and the userspace-`sh` prerequisite table; Stages 13/16/19/22/23/24
notes updated; `docs/progs.md` (+ a shell section or `docs/shell.md`); r09-r11 untouched and still building.
**Acceptance demo (run by hand at Step 13):** boot to `> `; `cd bin`, `pwd`, `ls`, `cd /`; type a long command, edit it
mid-line, recall history with Up; `echo hello | cat`; `ls > listing.txt`, `cat listing.txt`; `echo more >> listing.txt`;
`mkdir work`, `cd work`, `./../cdbin.sh` and `pwd` (scoped) vs `source ../cdbin.sh` and `pwd` (unscoped); `cp`, `mv`,
`rm -r work`; `chmod +x` on `binary256` then run it -> `cannot execute binary file: Exec format error`; `crash` -> `exit 139` and the shell continues;
`cat` reading typed lines until Ctrl+D; type while `spin 3` runs and see the keys arrive afterward; reboot QEMU with the same
image and confirm files created before are still there.
**Known limitations (stated in docs):** finite, sequentially executed pipelines only (Stage 23 for streaming/concurrency); no variables,
`$?`, `exit`, `if`/`for`, functions, globbing, `;`/`&&`/`||`, background jobs, here-documents, or fds above 2 until later
stages (the `exit N` line stands in for `$?`); `cd` with no operand goes to `/` until `$HOME`; Ctrl+D at the prompt does nothing; single resident program; kernel-resident shell (path to userspace `sh` recorded); FAT16 root has 512 slots.

## Kernel heap growth: what actually limits it (researched from r11's code and build)
- **Today:** heap = 1 MiB `static mut` in `.bss`; kernel image is ~200 KB text + 5.25 MiB bss (2 MiB virtio DMA pool,
  1 MiB heap, 1 MiB stack, rest small). Memory map: RAM starts `0x40000000`; kernel budget to `0x41000000` (16 MiB);
  unmapped guard gap to `0x44000000`; user window `0x44000000-0x44200000`; above it free RAM that Stages 17/18
  intend to hand to programs. QEMU `virt`'s default RAM (no `-m` given in the justfile or harness) is 128 MiB, i.e.
  up to `0x48000000`.
- **The hard ceiling is the fixed user address, not QEMU:** every user binary (all stages, shared `user/progs/link.ld`)
  is linked at `0x44000000`, so the kernel image (heap included) must end below it. With a healthy guard that allows
  roughly 30-40 MiB of heap. `-m` only adds RAM *above* the user window (headroom for Stages 17/18 -- and needs the
  flag added to both the justfile and `run_tests.py`); it doesn't raise this ceiling. QEMU allocates guest RAM
  lazily on the host, so a big RAM size costs nothing until touched (only the zero-filled `.bss` blob of the image
  is materialized at load).
- **Runtime growth is possible but not needed:** `linked_list_allocator` has `extend`, but `aarch64-paging` allocates
  page-table pages from the global allocator (mapping while extending risks re-entering it under the lock), and the
  region is already free. Simplest correct approach: one larger static heap (mapped for free by the existing
  `__data_start..__kernel_end` mapping), no runtime growth.
- **Allocator behavior to design around:** a `Vec` doubling needs old+new alive at once (peak ~1.5-2x), the
  free-list allocator fragments, and the default allocation failure is a panic -- so pipe buffers use
  `try_reserve` and a cap (e.g. 8 MiB), and reading a whole ELF (`read_file_checked`) stays fine at 16 MiB.
- **Comparison:** `disk.img` is 16 MiB, so a temp-file pipe is bounded by roughly the same size (and by the FAT free
  space), just with EIO instead of a cap error. A 16 MiB heap therefore gives equal capacity to the temp-file
  design, faster and without cleanup, while also benefiting large ELFs and directory snapshots.
- **Decision:** grow to 16 MiB in Step 1 (cheap, independent win: large ELFs, directory snapshots, future stages);
  pipes still use temp files. Any new large kernel allocation should use `try_reserve` and report an error rather than
  rely on the allocator not failing.

## POSIX alignment review (Cygwin-style: match where cheap, state deviations)
| Area | POSIX behavior | Plan | Deviation / reason |
|---|---|---|---|
| Redirection order, `2>`, `>>`, `2>&1`, builtins redirectable, earlier redirects stick on failure | left to right, applies to builtins | matched (Steps 6-8) | fds >2, here-docs unsupported |
| Quoting, `#` only at word start, `\$` in `"..."` | POSIX shell grammar | matched (Step 7) | no expansion/globbing until later stages |
| Exec of non-ELF | `ENOEXEC` -> run as `sh` script | matched (Step 9); binary -> error like bash | `#!` ignored (one interpreter) |
| `./script`, `sh script` | new shell process | pushed frame (`with_scope`) | no real process until Stage 19+ |
| `source`/`.` | current shell | no push; redirect via `with_stdio` | -- |
| Pipelines | concurrent; every stage runs; status = last | every stage runs, status = last, pipe bound before redirects | sequential via temp files |
| Command not found / not executable | exit 127 / 126 | bash wording; `exit N` line shows the pipeline status | no `$?` until variables |
| `cd` no operand / `cd -` | `$HOME` / `$OLDPWD` | `/` / unsupported | no env until Stage 16 |
| `pwd` | builtin + utility, `-L/-P` | program via `getcwd`; no flags | no symlinks |
| `mkdir`, `rm`, `mv` | multi-operand, continue on error, `rm` refuses `.`/`..`, `mv` into dirs | matched | no `-p`/`-f`/`-i`; `rm` on read-only file refuses instead of prompting |
| Canonical tty | Backspace, Ctrl+U, Ctrl+D partial flush/EOF, no arrows | matched (Step 12) | -- |
| Interactive shell EOF | exits on Ctrl+D | ignored | the shell is init |
| Signals / Ctrl+C / job control | yes | none | Stages 20-22 |
| `cat -` (existing, Stage 11) | `-` means stdin | unchanged, documented lone `-` = filename | pre-existing; revisit with a later `user/progs` pass |

## Decisions record
Decided while planning; later Steps may refine these but shouldn't silently reverse them.
1. **The shell stays kernel-resident** in Stage 12 (as the roadmap says); the path to a userspace `sh` is documented
   (see the prerequisite table in Step 13 and ROADMAP.md's Stage 12/19 text), not built.
2. **Each stage is its own codebase:** no shared kernel crate across `rNN_` directories. The `abi` crate is shared only
   with `user/`, which every stage already shares, and is additive.
3. **Tests belong to this stage** (`disk/tests/`, `test/progs/`); `user/` gets only core utilities (`pwd`, `mkdir`, `rm`, `mv`).
4. **Disk:** 64 MiB FAT16; `r12_shell/disk.img` is gitignored (r06-r11's stay tracked).
5. **Heap:** the kernel heap grows from 1 MiB to 16 MiB in Step 1 (a static in `.bss`, mapped for free; the real
   ceiling is the fixed user address `0x44000000`, not QEMU's RAM -- see the analysis below).
6. **Pipes:** temp files in `/tmp` (the ROADMAP's MS-DOS-style design), not a kernel-heap buffer.
7. **Prompt:** stays `> `; the cwd is not shown.
8. **Redirection:** `<`, `>`, `>>`, `2>`, `2>>`, `2>&1`/`>&2`, on builtins too (POSIX-style, via `with_stdio`);
   no fds above 2, no here-documents.
9. **`rm -r` drills into directories;** plain `rm` refuses them; no `rmdir` or `-f` for now.
10. **POSIX alignment:** non-ELF exec uses bash's `ENOEXEC` fallback (text -> script, binary -> `cannot execute binary
    file`); canonical-mode Ctrl+D delivers a partial line (Step 12); bash error wording (Step 7); the other rows of the
    POSIX table are applied as written.
11. **Names:** POSIX names throughout -- builtin `cd`, syscalls `getcwd` (now) and `chdir` (number reserved, not
    implemented until state is per-process).
12. **Unicode console:** GNU Unifont through the `unifont` crate (Step 2b), compiled into the kernel; BMP only -- the
    astral plane draws U+FFFD; Spleen and `cp437.rs` are dropped; widths come from the glyph (8 or 16 px) and the console
    gains a cell grid so Backspace, `show_row` and the line editor count cells. Rejected: pure CP437 (would have needed
    transcoding at every `&str` edge -- filenames, `Args`, `write_str`, the UART mirror) and a home-made range-indexed font
    file (recorded as the upgrade path in Step 2b).

## Files touched
`rust/r12_shell/src/{main.rs,elf.rs,fd.rs,files.rs,syscall.rs,process.rs,stdin.rs,input.rs,argv.rs,line.rs,console.rs,vectors.s}`,
new `shell_state.rs`, `shell.rs`, `lexer.rs`, `line_discipline.rs`; `rust/r12_shell/test/run_tests.py`, `justfile`, `disk/` fixtures;
`rust/user/{userlib,abi}` and `rust/user/progs/src/bin/{pwd,mkdir,rm,mv}.rs` (core utils only);
`rust/r12_shell/test/progs/` (test programs) and `rust/r12_shell/disk/tests/` (fixtures); `rust/docs/progs.md`;
`r12_shell/test/README.md`; `.gitignore`; `ROADMAP.md` (Stage 12 summary + forward-connection edits); `Stage12.md` (new, full plan).

## Verification
Per Step: `just test` in `r12_shell/` plus the Step's new cases; per Phase: rebuild r09-r11 (`just build`/`just test` in r11) to prove
`user/` changes stayed compatible; final: manual run (`just run`) on the virtio-gpu display for cursor/history feel.
