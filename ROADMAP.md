# Roadmap: a minimal Rust-based system with a filesystem and user programs

**Target:** a bare-metal AArch64 system, written in Rust, that can load a file
from a real filesystem, run it as a genuine EL0 user program (separate from
the kernel via real syscalls, not just a function-pointer table), and -- as a
capstone -- run a serial-console shell and a small vi-like full-screen editor
as user programs. Deliberately staying on the serial UART throughout --
no framebuffer, no graphics -- since none of the filesystem/user-program work
depends on the output device. Still single-threaded
throughout: one program loaded and run to completion (or until it exits) at a
time, no scheduler, no preemption, no concurrency.

This replaces the earlier shell-focused roadmap. The completed C stages
(`01_hello` through `04_dots`) stay in the repo as reference -- the concepts
(UART, GIC, exception vectors, the generic timer) all carry forward directly
-- but this roadmap's own stages are a fresh build in Rust, since the
toolchain and target are different enough that it isn't really a continuation
of the same numbered sequence.

Every crate named below was checked to actually exist, be `no_std`-compatible,
and fit this use case before being included here -- not assumed from memory.

---

## Stage 1: Rust bare-metal foundation -- `r01_hello`

**Goal:** the Rust equivalent of `01_hello` -- prove the toolchain, target,
and boot path work before building anything on top.

**Features:**
- `aarch64-unknown-none` target (an official Tier 2 target supporting
  `core`+`alloc` for `#![no_std]`).
- `#![no_std] #![no_main]`, a `#[panic_handler]`.
- A boot stub is still needed as hand-written assembly -- Rust doesn't remove
  this requirement, it's usually a small `global_asm!` block (or a `.S` file
  built alongside, same as `start.S`) doing exactly what `01_hello/start.S`
  does: set up `sp`, jump to a Rust `extern "C" fn kernel_main`.
- A linker script, structurally the same as `01_hello/link.ld`.
- Polled UART output (PL011 register access, same addresses/bits already
  confirmed via device-tree dump in `01_hello`).

**Demo:** boot and see `Hello, world!` printed once, in color -- byte-for-byte
the same observable behavior as `01_hello`, now from Rust.

---

## Stage 2: Exceptions and interrupts -- `r02_interrupts`

**Goal:** the Rust equivalent of `03_interrupts`, built on crates instead of
hand-rolled register pokes where a good crate exists.

**Features:**
- [`aarch64-cpu`](https://docs.rs/aarch64-cpu) for typed system-register
  access (`ELR_EL1`, `SPSR_EL1`, `VBAR_EL1`, `DAIF`, ...), replacing the raw
  `mrs`/`msr` inline asm scattered through the C version. This is the same
  crate used by
  [`rust-embedded/rust-raspberrypi-OS-tutorials`](https://github.com/rust-embedded/rust-raspberrypi-OS-tutorials),
  a parallel teaching series covering nearly this exact ground.
- The exception vector table itself is still hand-written assembly (same
  honest limitation as the boot stub above -- there's no portable, safe way
  in Rust to guarantee the exact code placement and lack of prologue the
  `ventry`/`kernel_entry` macros need).
- [`arm-gic`](https://github.com/google/arm-gic) (trustedfirmware.org
  -maintained, `no_std`, GICv2/v3/v4) replacing `registers.h`'s hand-rolled
  `GICD_*`/`GICC_*` definitions.
- UART RX wired through the GIC, same as `03_interrupts`.

**Demo:** the same interrupt-driven echo behavior as `03_interrupts`.

---

## Stage 3: Generic timer -- `r03_timer`

**Goal:** confirm timer access works through `aarch64-cpu`'s register
wrappers before anything depends on it.

**Features:** `CNTFRQ_EL0`/`CNTP_TVAL_EL0`/`CNTP_CTL_EL0` via `aarch64-cpu`,
wired to the GIC as PPI 30 (as already confirmed via device-tree dump in
`04_dots`).

**Demo:** reuse the `04_dots` idea as a quick sanity check -- type a digit,
see that many one-per-second dots -- just to confirm the timer primitive
works before it becomes load-bearing for later stages.

---

## Stage 4: Heap allocator -- `r04_alloc`

**Goal:** nothing before this stage needed a heap at all -- this is the first
genuinely new piece of infrastructure, not a port of something the C version
already had. Needed outright if Stage 6 picks `fatfs` over `embedded-sdmmc`
(the former requires `alloc`, the latter doesn't), and useful generally for
building up variable-length data -- a line of input, a path -- without a
fixed-size buffer.

**Features:** a `#[global_allocator]` (e.g. the `linked_list_allocator`
crate) over a static, fixed-size memory region reserved in the linker script.

**Demo:** an "echo line" program -- read UART input byte by byte into a
heap-allocated, growable buffer (`Vec<u8>`/`String`, growing as characters
arrive rather than a fixed-size array), stop at Enter, echo the whole line
back. Deliberately the first "accumulate input, then act on the whole line"
logic in this roadmap, not just an allocator smoke test: it's the same basic
shape Stage 8's minimal launcher needs (accumulate a line, then dispatch it),
which Stage 10's full shell builds on again with real editing on top.

---

## Stage 5: VirtIO block device -- `r05_blkdev`

**Goal:** a "disk" under QEMU, as the substrate the filesystem will sit on.

**Features:**
- [`virtio-drivers`](https://github.com/rcore-os/virtio-drivers) (`no_std`,
  with a working AArch64+QEMU example in its own repo) for the VirtIO
  transport and block-device driver.
- QEMU side, confirmed working syntax for the `virt` machine's MMIO
  transport (not the PCI variant, which would drag in a full PCI bus we
  don't otherwise need):
  ```
  -drive if=none,file=disk.img,id=hd0 -device virtio-blk-device,drive=hd0
  ```
- `disk.img` built on the host with `dd`/`truncate`, no filesystem on it yet.

**Demo:** read one known raw 512-byte sector from `disk.img` and print its
bytes over UART -- proving the block-device transport works in isolation,
before any filesystem logic sits on top of it and could mask a transport bug.

---

## Stage 6: Filesystem -- `r06_fs`

**Goal:** turn Stage 5's raw block device into files and directories.

**Features -- an explicit choice between two real options:**
- [`embedded-sdmmc`](https://github.com/rust-embedded-community/embedded-sdmmc-rs) --
  pure `no_std`, **no `alloc`** required, FAT16/32 without long filenames.
  Needs only a `BlockDevice` trait impl wrapping Stage 5's VirtIO driver.
  The lighter-weight starting point, and doesn't need Stage 4 at all.
- [`fatfs`](https://github.com/rafalh/rust-fatfs) -- more complete (long
  filenames, a more `std::fs`-like API), but requires `alloc` -- so it's
  "free" once Stage 4 exists anyway, which it will by the time `cosmic-text`
  needs it.

Recommendation: start with `embedded-sdmmc` for this stage specifically (it's
simpler and doesn't entangle the allocator with filesystem correctness), and
reconsider `fatfs` later only if long filenames turn out to matter.

**Demo:** format a small FAT image on the host (`mkfs.fat`), list the root
directory over UART, read a known file's contents and print them.

---

## Stage 7: EL0 and user programs -- `r07_userspace`

**Goal:** the biggest single stage in this roadmap -- real user/kernel
separation, not the function-pointer dispatch table the old shell-focused
roadmap planned. **Explicitly still single-threaded**: one user program
loaded and run at a time, to completion or exit -- no scheduler, no
concurrent processes, no preemption. That's a deliberate scope boundary, not
a placeholder for "add a scheduler next."

**Features:**
- The MMU, turned on for the first time anywhere in this whole project --
  needed for genuine EL0/EL1 memory separation. Page tables covering kernel
  memory (RWX as appropriate) and a separate user region.
- The `sync_el0_64`/`irq_el0_64` vector entries -- stubbed to
  `unexpected_exception` since `03_interrupts` -- now populated for real.
- `SVC`-based syscalls, a minimal set to start (`write`, `read`, `exit`).
- A minimal ELF loader: parse a simple, statically-linked binary read from
  Stage 6's filesystem, map its segments into the user region, `eret` into
  EL0 at its entry point.
- ([jcomes.org's "From Scratch: An AArch64 OS in Rust"](https://jcomes.org/aarch64-os-hello_world)
  covers similar ground on exception levels and early EL1 entry -- worth
  reading as a reference, not following lock-step.)

**Demo:** a tiny statically-linked "user" binary, stored on the Stage 6
filesystem image, that calls the `write` syscall to print
`hello from userspace` -- loaded from disk, launched at EL0, its output
observed arriving back through the syscall path over UART.

---

## Stage 8: a minimal program launcher -- `r08_repl`

**Goal:** the bare mechanism Stage 9's utilities need to be individually
testable at all -- Stage 7's demo got away with no selection mechanism
because there was only ever one program to run. "Type a name, find it on
disk, load it, run it" is unavoidably shell-*shaped* functionality, but it's
a hard prerequisite here, not the shell itself -- deliberately not the same
stage as real line editing, history, or pipes; those are Stage 10, once
Stage 9 gives them something worth piping.

**Features:**
- A bare line-reading loop -- no `noline`, no history, no cursor movement,
  same category of code as `02_echo`'s original polling loop -- accumulating
  bytes into the heap-allocated growable buffer Stage 4's "echo line" demo
  already proved, stopping at Enter.
- A tokenizer splitting the line into a program name + arguments (the same
  tokenizer Stage 10's full shell reuses/extends later). The remaining
  tokens after the program name are exactly what becomes `argv`, below.
- Stage 7's ELF loader invoked directly on the named file, read from Stage
  6's filesystem.
- **`argc`/`argv` setup, worked out in detail beforehand rather than
  improvised at implementation time:**
  - The tokenizer's output is held in Stage 4's heap, on the kernel (EL1)
    side -- there's no EL0 heap involved, and can't be: at this point the
    new program hasn't run a single instruction yet, including whatever it
    might use to set up a heap of its own.
  - Starting from `stack_top` (Stage 7's already-mapped EL0 stack region) and
    working *downward* -- the direction the stack grows, established all the
    way back in `01_hello/link.ld` -- the kernel writes each argument string,
    then the pointer array, into that memory, tracking where its write-cursor
    ends up as it goes. This is the kernel doing address bookkeeping on the
    target memory; the CPU's actual `SP` register hasn't been touched yet.
  - **The gotcha worth naming explicitly**: each pointer in the array must be
    written as that string's EL0-stack address, not left as the kernel-side
    address the string started at in Stage 4's heap -- an easy, silent
    mistake that would only surface the moment the program dereferences
    `argv[1]`.
  - Once everything is written, wherever the write-cursor ended up (the
    lowest address reached) becomes the value set as the new program's
    initial `SP` -- the write happens first, at addresses *above* where `SP`
    will end up; setting `SP` to its final value is the last step, not
    something done incrementally alongside the writes. Only then does `eret`
    happen, with `argc` and a pointer to the array passed via `x0`/`x1`.
- `echo` itself, built as part of this stage rather than deferred to Stage 9:
  argc/argv setup is meaningless to demo without a program that actually
  consumes its arguments, and "print back what I was given" is the minimal
  possible such program. It becomes the first of Stage 9's utilities in
  practice, just built one stage early because it's what proves this stage's
  actual subject matter works.

**Demo:** type `echo hello world` and press Enter -- the launcher tokenizes
the line, loads `echo` via Stage 7's loader, copies `hello` and `world` onto
its EL0 stack with a correctly-rewritten pointer array, passes `argc`=2 and
`argv` via `x0`/`x1` -- and `echo` prints `hello world` back, proving the
whole pipeline end to end, not just that a named program can be loaded at
all.

---

## Stage 9: mini-busybox utilities -- `r09_busybox`

**Goal:** a handful of genuine external EL0 programs to exec -- named after
[BusyBox](https://busybox.net/), the real-world project built on exactly this
premise (a small bundle of minimal Unix-utility implementations for
constrained/embedded systems). Sequenced *before* the full shell rather than
alongside it: Stage 10's pipes/redirection demo is far more convincing piping
real, independently-useful programs together than it would be inventing
throwaway test binaries just to prove the plumbing works.

**Features:**
- Extends Stage 7's minimal `write`/`read`/`exit` syscall set to
  `open`/`read`/`write`/`close` -- needed by everything here (`echo` already
  exists from Stage 8, built there specifically to prove `argc`/`argv`
  setup, and needed none of these). `ls` additionally needs some way to
  enumerate directory entries -- a dedicated `readdir`-style syscall, or
  treating a directory as a readable pseudo-file of raw entry records, are
  both reasonable; worth deciding when this stage is actually reached rather
  than committing now.
- `cat`, `ls`, `cp` as real, independent programs -- each one loaded through
  Stage 7's ELF loader like any other, not special-cased, and each one a
  real consumer of the `argc`/`argv` mechanism Stage 8 built for `echo`.
- **An open design question worth naming rather than silently deciding**:
  separate small binaries (simpler, each one a clean standalone exercise of
  the loader) vs. one true multi-call "busybox" binary that dispatches on how
  it was invoked (`argv[0]`) -- authentic to the name, but needs the shell
  and/or filesystem to support multiple names resolving to the same file
  (traditionally via symlinks), which is new surface area of its own.

**Demo:** run each new utility via Stage 8's minimal launcher: `cat` on a
known file prints its contents; `ls` lists the root directory read via
Stage 6's filesystem; `cp` copies a file and the copy's contents are
confirmed to match -- alongside `echo`, already working since Stage 8.

---

## Stage 10: a real shell, with pipes -- `r10_shell`

**Goal:** upgrade Stage 8's bare launcher into something worth typing at
regularly, now that Stage 9 gives it real programs worth combining -- not a
rebuild from scratch, an extension of the same tokenizer and ELF-loader
invocation Stage 8 already established.

**Features:**
- Real line editing with history, replacing Stage 8's bare polling loop:
  either hand-built, or via [`noline`](https://github.com/rustne-kretser/noline)
  (`no_std`, `embedded_io::Read`/`Write`-based, no-alloc static-buffer
  construction confirmed via its own source -- see the earlier investigation
  into its `input.rs` state machine). Note from that investigation: `noline`
  owns the raw byte stream itself and does its own internal parsing -- there's
  no seam to bolt an external parser like `vte` onto it, nor any need to.
- Redirection (`>`/`<`): close to free, given Stage 9's `open`/`read`/
  `write`/`close` syscalls already exist -- `cmd > file` is just "open the
  file, hand its descriptor to `cmd` as stdout."
- **Pipes (`cmd1 | cmd2`), via temp files -- deliberately not true streaming
  concurrency.** Real pipe semantics need two processes actually running at
  once, which Stage 7 explicitly rules out. Early MS-DOS hit this identical
  single-tasking wall and solved it the same way we will: run `cmd1` to
  completion with stdout redirected to a temp file on Stage 6's filesystem,
  then run `cmd2` to completion with stdin redirected from that file, then
  delete it. **Named limitation, not a bug to fix later:** this only works
  for pipelines whose stages produce a *finite* amount of output that fits on
  disk -- no infinite/streaming pipelines under this design, ever.

**Demo:** a `> ` prompt with working cursor movement and command history;
`echo hello | cat` and `ls > listing.txt` both work via the temp-file
mechanism above, using Stage 9's real utilities rather than synthetic test
programs; control returns to the prompt once each stage exits (via the
`exit` syscall from Stage 7).

---

## Stage 11 (capstone): a vi-like full-screen editor -- `r11_editor`

**Goal:** genuinely harder than the shell's line editing, not just a bigger
version of it -- a full-screen editor needs *both* directions of the VT100
vocabulary at once (parsing incoming keys, *and* driving outgoing
cursor-positioning/clear-region escapes to redraw an arbitrary part of a
multi-line screen), plus a problem the shell never faces at all: knowing how
big the screen even is.

**What already exists, checked directly rather than assumed:**
- No existing `no_std` vi-like editor was found. Real Rust vi-style editors
  exist (`kiro-editor`, `kilo-rs`, `iota` with `--vi`, `amp`) but all are
  hosted-OS programs built on `crossterm`/`termios` -- not portable to a
  freestanding target without rewriting their entire I/O layer. The original
  `kilo` editor (and these Rust ports of it) is still worth reading as a
  *design reference* -- around 1000 lines, built directly on raw
  terminal+ANSI with no TUI framework -- just not something to depend on
  directly.
- [`ratatui`](https://ratatui.rs/) gained real `no_std` support in v0.30
  (tested on ESP32/STM32H7/PSP/UEFI backends per its own release notes) --
  genuinely useful for the layout/widget/buffer-diffing half of this problem.
  But per ratatui's own docs, `no_std` mode requires writing a custom
  `Backend` ourselves (the built-in `crossterm`/`termion` backends are
  `std`-only), and **its `no_std` support says nothing about keyboard input
  at all** -- that half of the problem is unaddressed by ratatui regardless
  of backend.
- [`anes`](https://github.com/qwandor/anes-rs) provides both ANSI sequence
  *generation* (cursor movement, colors, via `Display`) and, behind a
  `parser` feature flag, sequence *parsing* -- co-maintained by the same
  Google bare-metal Rust contributor behind `virtio-drivers` and `arm-gic`,
  which is a good sign, but it doesn't declare `#![no_std]` explicitly, so
  treat it as "plausible, verify with an actual build against
  `aarch64-unknown-none` before relying on it" rather than confirmed.
- **Conclusion:** no combination of existing crates removes the need to write
  our own input parser -- that part is unavoidably new code either way.
  Worth deciding once this stage is reached whether `ratatui`+a hand-written
  `Backend` is worth its dependency weight over just hand-rolling both
  directions with `anes` (or nothing at all) -- a real, open choice, not
  settled here.

**New features specific to this stage:**
- Terminal-size discovery: no crate exists for this (checked). The
  `ESC[18t` query / `ESC[8;rows;colst` response round-trip, sent to the
  terminal and parsed back out of the same UART RX stream everything else
  arrives on.
- A multi-line in-memory buffer, modal editing (insert vs. command mode, at
  minimum), and screen-region redraw logic driven by cursor-positioning
  escapes -- the genuinely new work this stage exists for.
- File access reuses the `open`/`read`/`write`/`close` syscalls already
  established in Stage 9 -- nothing new needed on that front, and the editor
  is launched the same way as any other program via Stage 10's shell.

**Demo:** launch the editor from Stage 10's shell against a file already
present on the disk image, edit its text on-screen, save it, then -- to prove
persistence, not just an in-memory illusion -- restart QEMU against the same
`disk.img` and confirm the edit is still there.
