# r12_shell tests

`just test` runs the host tests (`just test-host`, plain `cargo test` on the kernel's pure-logic modules) and then the
QEMU tests (`just test-qemu`, which boots the kernel headless and drives it by typing on the virtio keyboard). This
directory holds the QEMU side: the runner (`run_tests.py`), the harness (`harness.py`), one module per area in
`cases/`, and the test-only programs below. How both layers work, where everything lives, and how to write a test are
described in [`docs/tests.md`](../../docs/tests.md); `just check-docs` (`check_docs.py`) also fails if a program, syscall or test program below is undocumented. The plan and per-Step test list are in `Stage12.md`.

## Test programs

The test-only EL0 programs in `progs/`, a separate Cargo package built by `just disk` and staged on the image as
`/tests/<name>.exe`:

| Program | Purpose |
|---|---|
| `probe` | Pokes at the syscall surface from EL0: `sys-unknown` (an unassigned syscall number), `bad-ptr` (bad/wrapping pointers and lengths to write/read/open/chmod), `fds` (opens files until refused, then closes them), `close-out` (closes fd 1 and reports on fd 2: run under `> f 2>&1`), `getdents-small` (`getdents` with a buffer under one record), `args ...` (prints argc/argv and what the stack layout guarantees). `frag`/`frag-raw` (a 200-fragment line through the stdout buffer vs. 200 raw writes -- the display flush counts differ), `interleave` (stdout `OUT`, stderr `ERR`, order kept), `bs-wide` (a wide glyph, backspace, `X`). Several more subcommands (`exit`, `poke`, `poke-w`, `user-ptrs`, `ioctl`, `getcwd`, `sp`, `stack`) are listed in `probe.rs`'s header. |
| `spin` | `spin N` busy-waits N seconds without reading anything, then prints `spun N` -- there is no sleep syscall, so this is how a test keeps a program running while it types. |
| `overflow` | Recurses without end, so it runs off the bottom of its stack: the kernel must stop it with a fault (`Segmentation fault ...`, exit status 139) and carry on. |
