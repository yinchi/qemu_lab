# r12_shell tests

`just test` runs both layers:

- **`just test-host`** -- plain `cargo test` in `hosttests/` on the host, for the kernel's pure-logic modules
  (path resolver, lexer, line editor, history, UTF-8 decoder, token queue, ...). A module qualifies by being
  `no_std` + `alloc` with no dependency on the rest of the kernel; `hosttests/src/lib.rs` pulls each one in by path.
- **`just test-qemu`** -- `run_tests.py` boots the kernel headless and drives it like a user would, typing on the
  virtio keyboard through the QEMU monitor's `sendkey` and checking the serial log (which mirrors the console), the
  display (`screendump`, where the serial log can't tell), and the disk image afterwards (`mcopy`, `fsck.fat -n`).
  It works on a sparse copy of `disk.img`, and fails at once on a `Kernel Panic!` or `Unexpected exception`.

  - `harness.py` -- the `Session` (QEMU + monitor + serial log), key names, `Context`, disk helpers.
  - `run_tests.py` -- runs every module in `CASES` in one session, then their `verify_disk` checks, then `fsck.fat -n`.
  - `cases/` -- one module per area, each with `run(ctx)` and optionally `verify_disk(ctx)`. `core_utils.py` is the
    regression baseline for the `user/progs` utilities; each Step of `Stage12.md` adds its own module.

The whole plan and the per-Step test list are in `../../../Stage12.md`.

## Where things live

Tests belong to this stage: everything test-only lives here or in `disk/tests/`, never in `user/`, which holds
only the core utilities every stage shares.

- `disk/tests/` -- static fixtures checked in (`hello.txt`, `data.bin`, `docs/example.txt`, `notes.txt`, and the
  scripts and trees later Steps add), plus the generated `*.exe` test programs
  and malformed ELFs (gitignored). It is on the disk image as `/tests/`. The kernel marks only `bin/` executable at
  boot, so the harness runs `chmod +x` on the programs it needs.
- `mkfixtures.py` -- run by `just disk`: derives `bigpad.exe` (a valid program plus 3 MiB of trailing zeros) and seven
  malformed ELF files (`elf-*.exe`) from `echo.exe`/`hello.exe`, so no binary blobs are checked in.
- `progs/` -- a separate Cargo package of test-only EL0 programs, built by `just disk` and staged as
  `disk/tests/<name>.exe`.

## Test programs

| Program | Purpose |
|---|---|
| `probe` | Pokes at the syscall surface from EL0: `sys-unknown` (an unassigned syscall number), `bad-ptr` (bad/wrapping pointers and lengths to write/read/open/chmod), `fds` (opens files until refused, then closes them), `args ...` (prints argc/argv and what the stack layout guarantees). Later Steps add subcommands. |
