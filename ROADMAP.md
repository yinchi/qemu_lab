# Roadmap: a minimal Rust-based system with a filesystem and user programs

**Target:** a bare-metal AArch64 system, written in Rust, that can load a file
from a real filesystem, run it as a genuine EL0 user program (separate from
the kernel via real syscalls, not just a function-pointer table), and -- as a
capstone -- run a shell and a small vi-like full-screen editor as user
programs, driven by a real VirtIO GPU display and a real VirtIO keyboard
rather than a serial terminal. Still single-threaded throughout: one program
loaded and
run to completion (or until it exits) at a time, no scheduler, no preemption,
no concurrency.

This replaces the earlier shell-focused roadmap. The completed C stages
(`01_hello` through `04_dots`) stay in the repo as reference -- the concepts
(UART, GIC, exception vectors, the generic timer) all carry forward directly
-- but this roadmap's own stages are a fresh build in Rust, since the
toolchain and target are different enough that it isn't really a continuation
of the same numbered sequence.

Stages 1-5 below stay serial-UART-only, exactly as originally planned and
already built: the boot path, exceptions, the timer, the heap allocator, and
the CSI-parsing, cursor-aware line editor are genuinely completed, working
infrastructure, and stay as-is. From Stage 6 onward, though, the interactive
path pivots away from a single bidirectional serial console (whether a real
UART or `virtio-console`) to a real, independent display device
(`virtio-gpu`, with a software-rendered text console built on top of its
pixel framebuffer) and a real, independent input device (`virtio-keyboard`)
-- the same "one virtual device per physical peripheral" property a
traditional PC has, chosen specifically because it removes the
terminal-emulator-mediated escape-sequence encoding that made Stage 5's CSI
parser and ESC-timeout logic necessary in the first place, and that would
otherwise have made the full-screen editor's hardest problem (blind,
escape-code-driven redraw of a 2D screen) harder still. (VGA text mode was
the original plan for the display half, for exactly this reason -- free
character-cell addressing and a hardware cursor, no rendering needed -- but
its legacy registers turned out to be unreachable on this platform, checked
both via a PCI BAR and the PCI I/O-space window; see Stage 6.) Stage 5
remains valid, demoable, completed work in its own right -- the same way this
roadmap's own predecessor C stages stayed in the repo as reference after an
earlier pivot -- it simply isn't extended by the stages that follow it.

Every crate named below was checked to actually exist, be `no_std`-compatible,
and fit this use case before being included here -- not assumed from memory.

From Stage 8 onward, wherever this roadmap layers Unix/POSIX-shaped behavior
(naming, display conventions, eventually `ls`/`cat`/`cp`, eventually
symlinks) over mechanisms that can't actually provide full POSIX semantics
(FAT has no inode indirection, so no hard links; nothing here is kernel-level
enough for a symlink to be transparent rather than an opt-in convention),
**Cygwin is the reference, not a target to fully match.** Cygwin -- a
POSIX-styled surface over NTFS/FAT with openly documented gaps, in real use
for 25+ years -- is proof this pattern works honestly: match POSIX styling
and behavior wherever it's cheap, and be explicit about where it isn't,
rather than either overclaiming compatibility or reverting to a DOS-native
style instead (checked, not assumed: "DOS with long filenames, styled
consistently" isn't actually a real historical lineage either -- LFN was a
Windows-95 GUI-era addition, and classic DOS command-line tools mostly
stayed 8.3-only for their whole working life). Each stage that touches this
boundary decides its own scoped subset of what to actually implement --
this is a standing design reference, not a commitment to Cygwin feature
parity.

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
already had. Needed outright if Stage 8 picks `fatfs` over `embedded-sdmmc`
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
shape Stage 10's minimal launcher needs (accumulate a line, then dispatch
it); Stage 5 extends this same buffer with real cursor-aware editing.

---

## Stage 5: Cursor-aware line editing -- `r05_lineedit`

**Goal:** "echo, but with a movable cursor" -- prove real single-line editing
(not just Stage 4's append-and-backspace-at-the-end model) works in
isolation. Follows the same pattern as Stage 3 proving the timer in isolation
before it became load-bearing infrastructure: get the fiddly part right on
its own, in the smallest possible program, rather than debugging it entangled
with a tokenizer, pipes, or modal editing state. A direct extension of what
already exists (Stage 3's timer, Stage 4's heap-backed buffer) rather than
needing anything new from outside the project, which is why it comes right
after Stage 4 instead of waiting behind the block-device/filesystem work.

Deliberately not [`noline`](https://github.com/rustne-kretser/noline): its
`sync_editor.rs`/`async_editor.rs` both read the next byte via a plain
blocking/awaiting `read_exact`, with no timeout hook anywhere, and its
`input.rs` parser only decides a lone Escape byte *wasn't* the start of a
sequence once the next byte arrives -- there's no bound on how long that can
take. Pressing Escape alone, with nothing following, hangs `noline`'s
`readline()` indefinitely. That fails a specific design principle this stage
is built around: **every keypress should produce something visible, or
nothing at all, within a bounded time -- never an indefinite, invisible
wait.**

**Features:**
- Extends Stage 4's growable buffer with a cursor position, so inserts and
  deletes can happen anywhere in the line, not just at the end
  (`insert(cursor, c)`/`remove(cursor)` in place of `push`/`pop`).
- A small CSI/ANSI escape parser recognizing at minimum Left/Right arrow
  (`ESC [ C`/`ESC [ D`) and Delete (`ESC [ 3 ~`) as single compound actions,
  rather than as separate, individually-meaningless bytes.
- Stage 3's timer, reused for the first time for something other than its
  own demo: bounds how long a lone `ESC` byte is held pending disambiguation
  from the start of a longer sequence, so it always resolves -- as a
  standalone Escape keypress (shown in standard caret notation, `^[`) or as
  part of a recognized sequence -- within a short, fixed timeout instead of
  waiting forever.
- Redraw logic: on insert or delete, reprint the line's tail from the cursor
  onward, then reposition the terminal cursor back to where it belongs. The
  same general technique classic line editors (and, at a coarser 2D grain,
  `curses`' screen-diffing) use, chosen over relying on terminal-native
  Insert/Delete-Character escapes (`ESC[@`/`ESC[P`) for portability across
  whatever terminal emulator is actually connected.
- Deliberately no history yet.

**Demo:** an interactive single-line editor over UART -- type text, move the
cursor left and right and insert or delete mid-line, with the terminal
correctly redrawing around each edit; press Enter to echo the final buffer
content back exactly as it was displayed.

---

## Stage 6: VirtIO block device + VirtIO GPU display -- `r06_virtio`

**Goal:** the first of two stages establishing the new interactive-I/O
architecture: a real display device the CPU addresses directly, sourced from
data read off a virtual disk through its own dedicated transport -- proving
both pieces work, together, before Stage 7 does the same for input.

VGA text mode was the original plan here, for the free character-cell
addressing and hardware cursor real hardware would have given -- but its
legacy Sequencer/CRTC/Graphics/Attribute-Controller registers turned out to
be unreachable on this platform (checked both via a PCI BAR and the PCI
host bridge's I/O-space window; neither reached the device). `virtio-gpu`
replaced it: a real virtio device, discovered the same way as the block
device, with a plain pixel framebuffer instead of character cells -- text
rendering became this project's own responsibility instead of free hardware.

**Features:**
- [`virtio-drivers`](https://github.com/rcore-os/virtio-drivers) (`no_std`,
  with a working AArch64+QEMU example in its own repo) for the VirtIO
  transport and block-device driver, over the `virt` machine's MMIO
  transport:
  ```
  -drive if=none,file=disk.img,id=hd0 -device virtio-blk-device,drive=hd0
  ```
- `virtio-drivers`'s VirtIO GPU driver (`VirtIOGpu`), discovered via the same
  `virtio,mmio` slot-probing as the block device, negotiating a fixed
  640x480 resolution (matching classic VGA mode 0x11's dimensions -- an
  80x30 grid at this stage's 8x16 glyph size) that's never renegotiated at
  runtime:
  ```
  -device virtio-gpu-device
  ```
- A raw bitmap font: [Spleen](https://github.com/fcambus/spleen)'s
  DOS/VGA-format dump (`dos/spleen.raw`, BSD-2-Clause licensed), 256 glyphs x
  16 bytes each, one byte per pixel-row, indexed directly by character code
  -- committed as `font/spleen.raw`, with attribution in `font/NOTICE`, and
  used as the actual "file read off disk" this stage's demo exercises.
- A software text console (`console.rs`) built over a generic `Framebuffer`
  (a raw BGRA8888/`B8G8R8A8` pixel surface, agnostic to whether its memory
  came from `virtio-gpu` or a plain buffer, so the rendering logic could be
  validated in isolation first): `put_char`/`putc`/`clear`/`clear_row`/
  `move_cursor`/`size` -- the character-cell API real VGA text mode would
  have given for free, rebuilt entirely in software over a pixel buffer
  instead.

**Demo:** read the font off `disk.img` via the VirtIO block driver (8
sectors = 4096 bytes), then render every printable ASCII character
(`0x20`-`0x7E`) through the console onto the `virtio-gpu` framebuffer and
flush it to the display -- proving the block-device transport, the GPU
display path, and the font-rendering pipeline all work, end to end,
together.

---

## Stage 7: VirtIO keyboard -- `r07_keydetect`

**Goal:** the second of the two new-architecture stages -- a real,
independent input device delivering discrete key events, replacing the
terminal-emulator-mediated escape-sequence encoding that made Stage 5's CSI
parser and bounded-ESC-timeout logic necessary.

**Features:**
- `virtio-drivers`'s VirtIO input driver (`VirtIOInput`), modeled directly on
  Linux's `evdev` layer: events arrive as `{type, code, value}` triples, one
  per key press or release, with no partial-sequence ambiguity to resolve at
  all.
  ```
  -device virtio-keyboard-device
  ```
- A keymap/modifier-state layer: tracking which modifier keys (Ctrl, Shift,
  Alt) are currently held, and combining a keycode with that state into
  whatever the consumer needs -- new work, but with no timing window or
  lookahead the way Stage 5's ESC-disambiguation had to have.

**Demo:** press a key combination -- e.g. Ctrl+End -- and see it correctly
identified and displayed via Stage 6's console as `You pressed Ctrl+End`,
proving keyboard input and modifier tracking work end to end, independent of
any line-editing logic built on top of it later.

---

## Stage 8: Filesystem -- `r08_fs`

**Goal:** turn Stage 6's raw block device into files and directories.

**Filesystem crate:** [`hadris-fat`](https://crates.io/crates/hadris-fat) --
FAT12/16/32 with long-filename (VFAT) support, built on `embedded-io`, no
`std` needed (`default-features = false`, `features = ["alloc", "lfn",
"read", "write", "sync"]`). Not the original plan -- both crates named here
originally, `embedded-sdmmc` and [`fatfs`](https://github.com/rafalh/rust-fatfs),
were checked to exist and claim `no_std` support before being written down,
same standard as everything else in this doc, but `fatfs` turned out
unbuildable in a genuinely no_std configuration on a modern toolchain: its
published 0.3.6 depends on `core_io`, an abandoned polyfill crate whose build
script hard-panics on any rustc newer than ~2021 (a hardcoded compiler
commit-hash lookup table, no viable override). `embedded-sdmmc` remains a
real, lighter-weight alternative (pure no_std, no `alloc` at all) if
`hadris-fat`'s heavier dependency tree or `alloc` requirement ever becomes a
problem -- not needed here since Stage 4 already provides the allocator.

The block device is wrapped in a byte-addressable `Read`/`Write`/`Seek`
adapter (`fat_io.rs`'s `BlkIo`) over Stage 6/7's IRQ-driven `Blk`, capped to
one sector per call by design; `hadris-fat`'s own `read_exact`/cluster-chain
logic drives however many calls are needed for a larger request, with a
read-modify-write on the write side wherever a call doesn't land on a whole
sector -- unavoidable for FAT-table/directory-entry writes specifically,
since those sectors are shared across many files' metadata, not owned by one
file the way a data cluster is.

**Design paradigm:** the Cygwin reference from this roadmap's intro, applied
concretely -- POSIX-styled directory listing (closer in spirit to `ls -1F`
than DOS's `<DIR>`-suffixed columns) and lowercase, case-sensitive naming,
over FAT storage that can't back the parts of POSIX that would need real
inode indirection: no hard links, ever, on this filesystem. Symlinks, if a
later stage ends up needing them, are planned as Cygwin's own mechanism
verbatim -- a plain file whose content is `!<symlink>` followed by the
target path, the DOS `SYSTEM` attribute bit as a cheap candidate filter --
not a bespoke lookalike, and not kernel-transparent: only ever followed by a
program that chooses to (Stage 11's `cp`/`cat`, a future shell's exec
lookup), never by this stage's filesystem layer itself. Not implemented
here -- there's no consumer for it until a later stage gives it one.

**Executable bit:** FAT has no execute-permission bit at all -- DOS never
needed one, since `COMMAND.COM` dispatched purely on filename extension
(`.COM`/`.EXE`/`.BAT`), never on stored metadata. This stage claims the
other reserved attribute bit (`0x40`; `0x80` stays unclaimed) as this
project's own convention for it, applied to exactly one user (there's no
multi-user model here at all, so no owner/group/other distinction is even
meaningful) -- `hadris-fat`'s `FatVolume::set_attributes` sets it, `ls
-1F`'s `*` displays it. Deliberately a claim, not a guarantee, same as real
POSIX: `chmod +x` on a file full of garbage still makes `ls -F` show `*`
there too, and the real failure only ever surfaces later, at `execve()`,
as its own distinct error (`ENOEXEC`) -- fully decoupled from what `ls`
already displayed. This stage's demo marks a `.sh` file, which nothing on
this system can actually run (no interpreter exists, and none is planned),
specifically to demonstrate that split concretely: setting and displaying
the bit needs no loader to exist yet, and (once Stage 9's loader does
exist) actually enforcing it -- refusing to run an ELF file with the bit
cleared -- is a separate, later decision, not something this stage commits
to either way.

**Demo:** format a small FAT image on the host (`mkfs.fat` + `mtools`, no
loopback mount required -- see `r08_fs/justfile`), list the root directory,
mark a file executable via `hadris-fat`'s write path (its first real
exercise in this program -- everything above it only ever reads) and read
a known file's contents, printing all of it via Stage 6's display.

---

## Stage 9: EL0 and user programs -- `r09_userspace`

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
  Stage 8's filesystem, map its segments into the user region, `eret` into
  EL0 at its entry point.
- ([jcomes.org's "From Scratch: An AArch64 OS in Rust"](https://jcomes.org/aarch64-os-hello_world)
  covers similar ground on exception levels and early EL1 entry -- worth
  reading as a reference, not following lock-step.)

**Demo:** a tiny statically-linked "user" binary, stored on the Stage 8
filesystem image, that calls the `write` syscall to print
`hello from userspace` -- loaded from disk, launched at EL0, its output
observed arriving back through the syscall path onto Stage 6's display.

> [!WARNING]
> Due to an uncaught bug, the MMU is not actually activated until Stage 12. 

---

## Stage 10: a minimal program launcher -- `r10_repl`

**Goal:** the bare mechanism Stage 11's utilities need to be individually
testable at all -- Stage 9's demo got away with no selection mechanism
because there was only ever one program to run. "Type a name, find it on
disk, load it, run it" is unavoidably shell-*shaped* functionality, but it's
a hard prerequisite here, not the shell itself -- deliberately not the same
stage as real line editing, history, or pipes; those are Stage 12, once
Stage 11 gives them something worth piping.

**Features:**
- A new token-emission layer, on top of (not inside) Stage 7's keymap module:
  turns one raw key event plus the current modifier/lock state -- Stage 7's
  `KeyState` (held keys) and `LockState` (CapsLock/NumLock/ScrollLock toggles)
  -- into an actual character or control action, e.g. `KEY_A` becomes `'a'`
  or `'A'` depending on Shift/CapsLock, `Ctrl`+letter becomes a control
  character, Enter becomes the line terminator. This is the direct
  replacement for `02_echo`'s "read one UART byte" primitive, and the first
  place anything in this roadmap actually *acts on* Stage 7's held/lock
  state rather than just displaying it back (all Stage 7's own demo ever
  did). Deliberately a separate module from Stage 7's `KeyState`/`LockState`:
  those two are pure state-tracking, updated the same way regardless of who
  reads them; this one is interpretation, consumed differently by a shell
  (wants characters) than a raw-mode app might (wants keycodes directly) --
  keeping them apart keeps that seam available instead of baking one
  consumer's needs into the state layer itself.
- A bare line-reading loop -- no history, no cursor movement, same category
  of code as `02_echo`'s original polling loop, just now pulling characters
  from the token layer above instead of reading raw UART bytes --
  accumulating them into the heap-allocated growable buffer Stage 4's
  "echo line" demo already proved, stopping at Enter. This loop is
  deliberately the *only* place Backspace ever gets handled: it pops the
  buffer right here, before Enter is ever reached, rather than being
  forwarded as a raw control byte to whatever eventually reads fd 0 -- a
  real program's stdin should only ever see a finished line's printable
  bytes plus a trailing newline, the same way a real tty's canonical
  (cooked-mode) line discipline works. Stage 11's `Keyboard::read()` reads
  from this same completed-line buffer rather than reinventing its own
  key-to-byte encoding. Deliberately not Stage 5's cursor-aware editor:
  this stage is a hard prerequisite for testing utilities, not the
  polished interactive experience Stage 12 aims for.
- A tokenizer splitting the line into a program name + arguments (the same
  tokenizer Stage 12's full shell reuses/extends later). The remaining
  tokens after the program name are exactly what becomes `argv`, below.
- Stage 9's ELF loader invoked directly on the named file, read from Stage
  8's filesystem.
- **`argc`/`argv` setup, worked out in detail beforehand rather than
  improvised at implementation time:**
  - The tokenizer's output is held in Stage 4's heap, on the kernel (EL1)
    side -- there's no EL0 heap involved, and can't be: at this point the
    new program hasn't run a single instruction yet, including whatever it
    might use to set up a heap of its own.
  - Starting from `stack_top` (Stage 9's already-mapped EL0 stack region) and
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
- `echo` itself, built as part of this stage rather than deferred to Stage
  11: argc/argv setup is meaningless to demo without a program that actually
  consumes its arguments, and "print back what I was given" is the minimal
  possible such program. It becomes the first of Stage 11's utilities in
  practice, just built one stage early because it's what proves this stage's
  actual subject matter works.

**Demo:** type `echo hello world` on Stage 7's keyboard and press Enter --
the launcher tokenizes the line, loads `echo` via Stage 9's loader, copies
`hello` and `world` onto its EL0 stack with a correctly-rewritten pointer
array, passes `argc`=2 and `argv` via `x0`/`x1` -- and `echo` prints
`hello world` back onto Stage 6's display, proving the whole pipeline end
to end, not just that a named program can be loaded at all.

---

## Stage 11: mini-busybox utilities -- `r11_busybox`

**Goal:** a handful of genuine external EL0 programs to exec -- named after
[BusyBox](https://busybox.net/), the real-world project built on exactly this
premise (a small bundle of minimal Unix-utility implementations for
constrained/embedded systems). Sequenced *before* the full shell rather than
alongside it: Stage 12's pipes/redirection demo is far more convincing piping
real, independently-useful programs together than it would be inventing
throwaway test binaries just to prove the plumbing works.

**Features:**
- **One multi-bin Cargo package, one ELF per program -- not a single
  multi-call binary.** The open question this stage started with (separate
  small binaries vs. one BusyBox-style binary dispatching on `argv[0]`) is
  settled: `user/progs/` is a single package whose `src/bin/<name>.rs` files
  each build to their own standalone ELF, sharing one `Cargo.toml`, one
  `link.ld`, and a `src/lib.rs` of common helpers (`user/userlib/` stays the
  runtime underneath). A true multi-call binary would have needed multiple
  names resolving to one file -- traditionally symlinks, which nothing here
  implements yet (see Stage 8) -- and would have saved nothing on a 16 MiB
  disk; each program is still loaded through Stage 9's ELF loader like any
  other, not special-cased. Stage 9's and 10's `hello`/`crash`/`echo` crates
  were folded into this package. `docs/progs.md` is the living record of every
  program: its POSIX equivalent, exactly which subset of it is supported, and
  which stage added each feature.
- **The utilities:** `cat`, `ls`, `cp`, `head`, `tail`, `wc`, `hexdump`,
  `true`, `false`, `chmod` -- alongside `echo`, already working since Stage
  10. `tail` and `chmod` are additions to this stage's original list of three:
  `tail` was pulled forward from Stage 22 (it's a small variation on `cat`'s
  read loop), and `chmod` is the first program to write the `0x40`
  executable-bit convention (Stage 8) from userspace -- restricted to
  `+x`/`-x`/`+w`/`-w`, since FAT has no other permission bits. `cd`, `pwd`,
  `mkdir`, `rm` and `mv` are Stage 12's, not this stage's: `cd` has to be a
  shell builtin, `pwd` and relative paths need a working directory that only a
  shell can hold, and the rest need directory-mutating syscalls this stage
  doesn't add. Programs are installed on disk as `bin/<name>.exe`, with the
  launcher trying the bare name first and `name.exe` second (Cygwin's own
  lookup order); `.exe` here is a naming convention that makes the files
  recognizable as programs to a host inspecting `disk.img`, not something the
  loader consults.
- **New syscalls**, all with Linux aarch64 numbers borrowed for familiarity
  and our own argument conventions: `open` (a path, read or write -- a write
  open creates the file if needed and always starts it empty, with no append
  or seek, since `cp` is the only writer), `close` (which also commits a
  written file's size to disk), `getdents` (a dedicated directory-reading
  syscall, filling fixed-size records -- decided in favor of this over treating
  a directory as a readable pseudo-file), and `chmod`. Errors come back as a
  negated Linux errno (`errno.rs`); the kernel's pointer validation was also
  hardened to reject a `ptr + len` that wraps around the address space.
- `open()` needs somewhere to put its result: Stage 9's fixed `0`/`1`/`2`
  match in `fd.rs` is promoted into an actual `static mut` table of
  `Option<FileDescriptor>`, with a new `File(handle)` variant indexing
  `files.rs`'s open-file table (readers, writers, and snapshotted directory
  listings). `fd::reset_for_launch()`, called from `process.rs`'s
  `run_program` before each launch, refills the table with the standard three
  entries and closes anything left over; `fd::end_launch()` closes and commits
  whatever the program left open once it exits or faults. `cp` -- needing a
  source and a destination open at once -- is the first real exercise of more
  than the fixed three slots being occupied simultaneously.
- **`Keyboard::read()` (fd 0) blocks inside the syscall.** The plan for this
  stage originally was to read from Stage 10's completed-line buffer. That
  can't work as written: `run_program` masks every DAIF bit for a program's
  whole time at EL0, so no keyboard IRQ ever fires while a program runs, and
  nothing would ever complete a line for it to read. Instead `read(0)`
  drains the virtio-keyboard queue directly (which fills regardless of the
  mask) through the same token-to-`LineBuffer` path the prompt uses -- the
  shared piece factored out into `input.rs` -- so Backspace is still absorbed
  in exactly one place and a program only ever sees a finished line plus its
  newline, like a real tty's cooked mode. Ctrl+D on an empty line is
  end-of-file, which is what lets `cat` with no arguments stop. Keeps
  `Keyboard` and a `File(handle)` symmetric at the `read()` call `for_fd`'s
  dispatch treats uniformly, which is exactly what Stage 12's redirection needs.
- **A nonzero exit status is reported.** `exit`'s argument was previously
  discarded; it's now returned by `run_program`, and the launcher prints
  `exit N` for a nonzero status (`false` prints `exit 1`; a program stopped by
  a fault, `exit 139`, same as a shell would report a segfault) -- otherwise
  `true` and `false` would be indistinguishable.
- **Console output is mirrored to the UART.** Everything a program prints, the
  shell prompt, and each finished input line also go to the serial port (with
  `\r\n` line endings), so a serial log is a readable transcript of a session.
  This is what makes the automated test below possible. It applies to plain
  line-oriented console writes; a raw-mode full-screen program (Stage 13's
  editor) would bypass it.
- **More kernel memory.** The kernel stack grows from 16 KiB to 1 MiB and the
  heap from 128 KiB to 1 MiB (the kernel image has a 16 MiB budget and comes
  to about 6 MiB, mapped apart from user memory, so nothing is gained by being
  stingy). The new
  filesystem-write path is the deepest call chain in the kernel -- keyboard IRQ,
  through `launch`, into a syscall, down into `hadris-fat` -- and the stack sits
  directly above `.bss`, so overflowing it silently corrupts the heap rather
  than faulting; this showed up as an intermittent `virtio-drivers` assertion
  during `cp`, which 16 KiB and even 64 KiB stacks (with the old heap) hit.
- **Automated test.** `just test` (`r11_busybox/test/run_tests.py`) boots the
  kernel headless and drives it exactly as a user would -- typing on the
  virtio keyboard through the QEMU monitor's `sendkey` -- then checks each
  command's output against the serial log and the disk image's final contents
  (copied files byte-for-byte, attribute bits) after QEMU has exited. It works
  on a copy of `disk.img`.

**Demo:** run each new utility via Stage 10's minimal launcher: `cat` on a
known file prints its contents; `ls -F` lists the root directory read via
Stage 8's filesystem; `cp` copies a file (text and binary) and the copy is
confirmed to match, from the shell and from the host; `head`/`tail`/`wc`/
`hexdump` on the same fixture; `chmod -x` makes a program refuse to launch and
`chmod +x` restores it; `cat` with no arguments echoes typed lines back until
Ctrl+D -- alongside `echo`, already working since Stage 10. `just test` runs
all of this and more unattended.

---

## Stage 12: a real shell, with pipes -- `r12_shell`

**Goal:** upgrade Stage 10's bare launcher into something worth typing at regularly, now that Stage 11
gives it real programs worth combining -- an extension of the same tokenizer and ELF-loader invocation,
not a rebuild. `kernel_main` ends in the shell's read-eval loop and never returns, the role a Unix
kernel's `init` (PID 1) plays. The shell stays **kernel-resident** (never loaded through Stage 9's ELF
loader as an EL0 program); what it would take to move it to userspace is recorded below. This is the
largest stage so far, so its full plan -- Steps, per-Step tests, the state it ends in, the decisions
behind it and a POSIX-alignment table -- lives in [`Stage12.md`](Stage12.md); this section is the
summary.

**Phase 1 (Steps 1-5): fix and restructure what exists**, because reviewing Stages 9-11 found
things the shell shouldn't be built on:
- A malformed ELF no longer panics the kernel (Stage 11's `chmod +x` made that reachable): `load()`
  validates everything first and `launch` reports `cannot execute: Exec format error`. A command word
  containing `/` runs that path.
- One small `abi` crate (syscall numbers, errno values, dirent layout) shared by kernel and `userlib`
  instead of copies kept in sync by convention; unknown syscalls return `ENOSYS`, bad pointers `EFAULT`.
- The kernel heap grows from 1 MiB to 16 MiB (a static in `.bss`; the real ceiling is the fixed user
  address, not QEMU).
- **The MMU is turned on for real** -- the fix for the bug flagged under Stage 9. Stages 9-11 built their
  page tables and set `TTBR0_EL1` but never wrote `TCR_EL1` or `SCTLR_EL1.M`, so translation stayed off and
  none of the permissions, the EL0-only window or the "guard gaps" were ever enforced (an overflowing user
  stack ran straight through the program and into the kernel). Stage 12 configures `TCR_EL1`, sets
  `SCTLR_EL1` `M | C | I` plus WXN, stack-alignment checks and PAN, page-aligns every linker section, gives
  the user stack an explicit mapping with an unmapped guard, and makes the kernel check every user pointer
  against what is really mapped. Stages 9-11 are deliberately left as they were built. Kernel and user
  addresses stay identity mapped; only per-process address spaces (Stages 17-19) would change that.
- The console write path decodes UTF-8 across `write` calls (no more `<invalid utf8>` for binary output
  or a character split at a 4096-byte boundary), draws Unicode with GNU Unifont (Basic Multilingual Plane
  only; wide glyphs take two cells; wrapping follows xterm), and stops flushing the GPU per fragment.
- One line-discipline module replaces the two duplicated copies (the prompt's and `read(0)`'s).
- Long input lines wrap onto the following rows instead of scrolling sideways in a one-row window (a
  layout module shared with Step 12's cursor-aware editor); a line may not outgrow the screen.
- **The eval loop leaves IRQ context.** Today `handle_keyboard_irq` calls `launch`, so a program runs
  inside an unacknowledged interrupt -- which is why every DAIF bit is masked at EL0 and `read(0)` drains
  the device itself. Instead the IRQ handler only enqueues `Token`s onto a ring buffer, `kernel_main`'s
  loop consumes them, and programs run with interrupts enabled.

**Phase 2 (Steps 6-12): the shell**
- **Shell-state frames.** A stack of `{cwd, stdio triple}` (Stage 16 adds `env`) with two distinct
  operations: `with_scope` pushes a whole frame (a script's own scope) and `with_stdio` saves and
  restores only the stdio triple (every redirect, so a redirected builtin's `cd` still sticks).
  There is still one fd table, not one per program -- at most one program is ever resident, so the
  kernel-owned table is rebuilt from the top frame's stdio triple at each launch (Stage 19 revisits
  this).
- **Working directory:** `cd` (a builtin), `pwd` (a program over a new `getcwd` syscall), one path
  resolver with `.`/`..`. `mkdir`, `rm` (with `-r`) and `mv` as utilities over new directory-mutating
  syscalls (`hadris-fat` already has `create_dir`, `delete` and `rename`). `chdir` as a syscall is
  deliberately absent until state is per-process.
- **A lexer that keeps quoting information** (POSIX quoting; `#` only at word start), replacing
  `shlex::split`, which can't tell `echo "|"` from a pipe.
- **Redirection:** `<`, `>`, `>>`, `2>`, `2>>`, `2>&1`, applied left to right and on builtins too; no fds
  above 2 or here-documents.
- **Scripts are recursion, not processes** -- the interpreter calling itself. `source`/`.` runs against
  the current frame; `sh FILE` and `./FILE` push one, so a script's `cd` and redirects don't leak,
  as they wouldn't from a real child process. A non-ELF exec-bit file that looks like text runs as a
  script (bash's `ENOEXEC` fallback).
- **Line editing and history:** Stage 5's cursor-aware buffer (insert/remove at a position) without its
  CSI parser or ANSI redraw -- input arrives as discrete key events and redraws go straight through
  `put_char` -- a block-glyph cursor, Left/Right/Home/End/Delete, Ctrl+A/E/U/K, and a history ring on
  Up/Down. Programs' `read(0)` stays a tty-style canonical mode (Backspace, Ctrl+U, Ctrl+D).
- **Pipes via temp files, deliberately not true streaming concurrency.** Real pipes need two processes
  running at once, which Stage 9 rules out. As early MS-DOS did, `cmd1` runs to completion with stdout
  in a temp file under `/tmp`, then `cmd2` with stdin from it, then the file is deleted -- every stage
  runs, the pipeline's status is the last stage's, and the pipe is bound before a stage's own redirects,
  as in POSIX. **Named limitation, not a bug to fix later:** finite output that fits on disk only --
  no infinite/streaming pipelines under this design, ever (Stage 23 replaces the design, not this
  limitation).
- The disk image grows to 64 MiB FAT16 (its root directory stays a fixed 512 slots, so everything lives
  in subdirectories) and is gitignored from this stage on. Tests live in this stage's own `disk/tests/`
  and `test/progs/`, never in `user/`, which holds only core utilities; `just test` runs host-side unit
  tests for the pure-logic modules and a QEMU-driven suite that types on the virtio keyboard.

**Path to a userspace `sh`.** Not built here, but each prerequisite has an owner:

| A userspace `sh` needs | Provided by |
|---|---|
| The shell not running in IRQ context; one input queue independent of its reader | Stage 12, Step 5 |
| Per-process `cwd`, stdio bindings and `env`, inherited by a child | Stage 16 adds `env`; Stage 19's slots hold a process struct (Stage 12's frame is plain data so it can become that struct) |
| Two programs resident at once (the shell stays loaded while a child runs) | Stage 19, with Stages 17/18 for window sizing and heap |
| `spawn`/`wait` syscalls (the fork/exec equivalent) | Stages 19 and 22 |
| `chdir` (number reserved) and `dup2`-style fd control | new syscalls once state is per-process |
| Real pipes between resident programs; job control | Stages 23 and 20-22 |
| `./script` and `sh script` as a real child process | Stage 19+ spawn (Stage 12's `with_scope` is the same operation) |

**Demo:** a `> ` prompt on Stage 6's display; edit a long command mid-line and recall it with Up;
`cd bin`, `pwd`, `ls`; `echo hello | cat`; `ls > listing.txt`, `cat listing.txt`, `echo more >>
listing.txt`;
`mkdir work`, `cd work`, then a script that `cd`s run as `./s.sh` (the directory is unchanged afterward)
and as `source s.sh` (it changed); `cp`, `mv`, `rm -r work`; `chmod +x` on a binary file and run it
(`cannot execute binary file`); `crash` (`exit 139`, and the prompt is still there); type while a
CPU-bound program runs and watch the keys arrive afterward; restart QEMU against the same `disk.img` and
find the files still there. `just test` runs the automated version of all of this unattended.

---

## Stage 13 (Capstone 1): a vi-like full-screen editor -- `r13_editor`

**Goal:** genuinely harder than the shell's line editing, not just a bigger
version of it -- a full-screen editor needs a multi-line buffer and modal
editing on top of everything Stage 12 already has, even though Stage 6/7's
`virtio-gpu`+keyboard architecture removes what would otherwise have been
this stage's hardest problems: there's no terminal-size query needed (the
display's dimensions are fixed and known up front by this project's own
design, unlike a real serial terminal's `ESC[18t` round-trip), and no blind,
escape-code-driven redraw needed (the CPU can read back and rewrite any cell
in the framebuffer directly, so there's no ANSI vocabulary to speak in
either direction).

**New features specific to this stage:**
- A multi-line in-memory buffer, and modal editing (insert vs. command mode,
  at minimum) -- the genuinely new state this stage introduces; both are
  independent of the I/O model and would have been needed however Stages 6/7
  turned out.
- Screen redraw: rewriting the entire visible page's worth of character
  cells on every edit via Stage 6's console, then one `flush()` call to push
  it to the display -- no differential/region-tracking logic required, since
  a full-page rewrite plus a single flush is cheap regardless. Worth naming
  explicitly: unlike real VGA VRAM, `virtio-gpu` writes aren't automatically
  visible on screen -- `flush()` is what actually transfers the framebuffer
  to the display, one extra step real memory-mapped VRAM wouldn't have
  needed, though still just one cheap call per edit, not something to
  optimize.
- A visible cursor needs drawing ourselves, same as Stage 12's: rendered as
  an ordinary glyph via `put_char`, since `virtio-gpu`'s only cursor
  primitive is a 64x64 ARGB mouse-pointer overlay, not a character cell.
- [`kilo`](http://viewsourcecode.org/snaptoken/kilo/) (and its Rust ports,
  `kiro-editor`/`kilo-rs`) remains a useful design reference for the
  multi-line-buffer-plus-modal-editing structure itself -- around 1000 lines,
  easy to read end to end -- even though its terminal I/O layer (built on
  `termios`/ANSI escapes) isn't something this stage needs or borrows from.
- File access reuses the `open`/`read`/`write`/`close` syscalls already
  established in Stage 11 -- nothing new needed on that front, and the
  editor is launched the same way as any other program via Stage 12's shell.
- A toggleable raw-mode switch on fd 0 -- the one genuinely new syscall
  surface this stage needs. Off by default (Stage 10/11's canonical,
  line-buffered mode); once this editor switches it on, its own `read()` calls pop `Token`s straight from the token queue Stage 12 introduced, bypassing the line discipline entirely -- no
  Enter-wait, no Backspace-absorption, since the editor decides what
  Backspace means itself (delete-under-cursor, not "edit the pending
  line"). Switched back off on exit, restoring Stage 12's shell to
  canonical mode. Not `termios`/ANSI raw mode -- just this project's own
  version of the same cooked-vs-raw distinction, since Stage 7's
  `virtio-keyboard` already delivers discrete key events with nothing
  escape-sequence-shaped to negotiate. Nothing before this stage has
  anything to toggle it; the switch itself is worth having now regardless,
  so `tokens.rs`'s `Token` (already carrying raw evdev codes and modifier
  flags, not just resolved characters) has a real consumer to have been
  designed for.
- Flipping this switch on needs no interrupt-mask changes. Stages 10-11 masked every DAIF bit for a
  program's whole time at EL0 to close a reentrancy hazard (a keyboard IRQ re-entering
  `handle_keyboard_irq` while `run_program` was still on the stack), but Stage 12 removes the hazard
  instead of masking around it: the keyboard IRQ only enqueues `Token`s, the shell's loop runs outside
  IRQ context, and programs run with interrupts enabled. Raw mode is then purely a routing switch on
  that queue -- who consumes each `Token`, the shell's line discipline or this editor directly -- and
  keystrokes typed while a program isn't reading wait in the queue instead of being lost.

**Demo:** launch the editor from Stage 12's shell against a file already
present on the disk image, edit its text on Stage 6's display using
Stage 7's keyboard, save it, then -- to prove persistence, not just an
in-memory illusion -- restart QEMU against the same `disk.img` and confirm
the edit is still there.

---

## Beyond Capstone 1

Stage 13 is the first real checkpoint, not the finish line -- the project stays open-ended past
it. The two stages below are small, self-contained additions that don't block, and aren't blocked
by, Stages 10-13's own sequence; they're placed here rather than inserted earlier specifically to
avoid renumbering that already-written sequence.

## Stage 14: a real-time clock -- `r14_rtc`

**Goal:** the same "prove the primitive works in isolation before it's load-bearing" pattern
Stage 3 already used for the generic timer -- a new hardware peripheral, introduced on its own
before anything else depends on it.

**Features:**
- A minimal driver for the PL031 RTC: confirmed present on QEMU's `virt` machine and
  board-intrinsic (same category as the GIC/UART -- no `-device` flag needed, unlike the optional
  virtio peripherals), verified empirically via a direct register read (`RTCDR` at `0x0901_0000`)
  matching the host's own `date +%s` exactly. Just one register matters for a first cut: `RTCDR`
  itself, a read-only 32-bit count of seconds since the Unix epoch -- `RTCLR`/`RTCMR`/the
  interrupt-control registers exist for setting the clock or firing an alarm, neither needed here.
- A new syscall, shaped like the existing ones (no arguments, current epoch-seconds returned in
  `x0`) rather than inventing a new convention.
- `date`, a new read-only EL0 utility -- prints the current time via the new syscall. **No
  timezone support, by deliberate decision, not an oversight:** the raw RTC value is already
  timezone-independent (Unix epoch seconds are UTC by definition -- that's exactly what matched
  host time in the address above), and a real timezone (a zoneinfo/tzdata database, DST
  transition rules, a configurable `$TZ`) is a genuinely large subsystem, wildly disproportionate
  to what this stage needs. This system just runs in UTC, the same deliberate choice plenty of
  real minimal/server/embedded systems make. Printing the raw epoch-seconds integer is a
  sufficient first cut; human-readable calendar formatting (still UTC) is a nice-to-have on top,
  not required. Stage 16's environment variables are what would actually unlock this later --
  `$TZ` is ordinarily just one conventional variable riding on that general mechanism, not
  something that needs its own bespoke configuration path.

**Demo:** run `date` via Stage 10's launcher twice, a few seconds apart, and confirm the printed
value actually advances -- proving it's live, not a build-time constant.

---

## Stage 15: real file timestamps on save -- `r15_save_time`

**Goal:** replace the fixed, build-time `SOURCE_DATE_EPOCH` timestamp every file on `disk.img`
carries since Stage 8 with a genuine, RTC-sourced one, for any file a program actually writes at
runtime -- the first point since Stage 8 where a file's on-disk timestamp reflects something other
than that fixed build-time value.

**Features:**
- Stage 14's RTC driver, called from wherever `hadris-fat`'s write path currently leaves a
  written file's directory-entry timestamp at whatever default it has -- check `hadris-fat`'s own
  API for how a caller supplies a modification timestamp on write when this stage is reached.
- No new consumer needed: `cp` (Stage 11) and the editor's `save` (Stage 13) already write file
  content through `hadris-fat`; this stage only changes what timestamp those existing writes
  carry, not what writes files in the first place.

**Demo:** `cp` a file (or save from the editor), inspect its directory entry, and confirm the
timestamp reflects real "now" -- rather than the fixed build-time value every other file on the
image still carries.

---

## Stage 16: environment variables -- `r16_env`

**Goal:** give programs (and the shell that launches them) a genuine, general-purpose key=value
store, by extending Stage 10's `argc`/`argv` mechanism with the third array real `execve()` passes
alongside them -- `envp` -- rather than inventing a special-purpose configuration path each time
something new (like Stage 14's own deliberately-deferred `$TZ`) needs configuring.

**Features:**
- A third array, `envp`: `NULL`-terminated pointers to `"KEY=VALUE"` C-style strings, written onto the new program's stack the same way Stage 10 already writes `argv`'s strings and pointer array (through the `push_cstr_array` helper Stage 12 extracts from `run_program`),
  passed via `x2` alongside `argc` (`x0`)/`argv` (`x1`) -- extending Stage 10's own mechanism, not
  replacing it.
- The shell (Stage 12) gains `export KEY=VALUE` (updating an entry before the *next* launch) and
  passes its own current environment to every child it launches -- the parent-to-child inheritance
  real Unix gets for free from `fork()`, replicated here by hand since there's no `fork()` to
  inherit from automatically.
- **The environment joins Stage 12's shell-state frame -- no new stack.** Stage 12 already builds the
  frame stack (`{cwd, stdio triple}`, `with_scope` pushing a whole frame, `with_stdio` saving only the
  redirections); this stage adds an `env` field to that frame and nothing else. `./script.sh` and `sh`
  push a copy of the current frame, so anything the script exports stays local to it; `source`/`.`
  pushes nothing, so its `export`s persist in the caller -- the scoping split Stage 12 already
  demonstrates with `cd`, now covering environment variables too. `cd` with no operand switches from `/`
  to `$HOME`. Because the frame is plain data it is also what becomes a per-process struct once
  Stage 19 lets a child exist (see Stage 12's path to a userspace `sh`).
- `env`/`printenv`, a small utility printing the current environment -- cheap once the mechanism
  exists, matching Stage 11's other utilities' spirit.

**Demo:** four checks, each isolating one piece of the mechanism:
1. **Inheritance:** `export FOO=bar`, then run a program that reads its own `envp` and prints
   `FOO`'s value -- confirming the variable was actually passed down, not just set in the shell's
   own memory.
2. **Own scope:** `./run_script_with_own_scope.sh` containing `export BAZ=qux`; after it returns,
   confirm `BAZ` is *not* visible in the shell -- the pushed frame was popped back off, taking the
   script's own export with it.
3. **`source`:** `source run_script_with_own_scope.sh` (the same script, run the other way);
   after it returns, confirm `BAZ` *is* now visible in the shell -- no frame was pushed, so the
   export applied directly to the caller's own environment, exactly as if typed at the prompt.
4. **The door this reopens:** a `date` reading `$TZ` (even just a fixed UTC offset, no
   DST/zoneinfo database needed) and adjusting its printed output accordingly.

**A verified, real upgrade path beyond the fixed-offset default, if ever wanted:** the
[`chrono-tz`](https://github.com/chronotope/chrono-tz) crate supports `no_std`
(`chrono`/`chrono-tz` both with `default-features = false`) and embeds the full IANA database,
historical DST transitions included -- genuine `$TZ`-aware local time, not just a fixed offset.
Its own README warns that "the additional binary size added by this library may overflow
available program space" on a real microcontroller; that specific risk doesn't apply to us (a
QEMU-emulated host with generous RAM, not a flash-constrained MCU), but it still has one concrete
consequence worth naming here rather than discovering later: `mmu.rs`'s fixed 2 MiB user window
(`USER_SIZE`) was sized for tiny, few-KB demo binaries, and a program statically linking a full
timezone database would be the first thing to actually pressure-test that budget -- likely still
fine, but worth deliberately re-checking (or enlarging the window) when this is reached, not
assuming it fits by default.

---

## Stage 17: arbitrarily large binaries -- `r17_large_binaries`

**Goal:** let a program's footprint use as much RAM as it actually needs, up to what's genuinely
free -- not the small, uniform ceiling every program has been held to since Stage 9. Directly
motivated by Stage 16's `chrono-tz` aside: a program linking a full timezone database needs
meaningfully more than `hello`/`crash` ever did, and paying that same cost for every program
regardless of need is the wrong trade.

**Features:**
- `elf.rs`'s `load()` computes the ELF's actual footprint (the highest `p_vaddr + p_memsz` across
  every `PT_LOAD` segment) before mapping anything, instead of checking each segment against a
  single fixed `USER_SIZE` constant.
- `mmu.rs`'s `USER_SIZE` becomes a ceiling, not the amount actually mapped -- the mapped extent
  for a given load is whatever `elf::load()` just computed for that specific program. Still
  identity-mapped, still growing from the same fixed `USER_BASE` (every user binary is linked at
  that same address; growing the window doesn't mean giving up the fixed base), still no physical
  frame allocator needed -- the extra room is already sitting there, deliberately unmapped, in the
  gap between the kernel's region and the top of RAM.
- **Shrink-on-load, not just grow-on-load:** the one genuinely new correctness requirement
  variable-sized windows introduce. If program A needed 500 KB and program B (loaded next) only
  needs 10 KB, B must not inherit A's leftover 490 KB still marked `USER`-accessible -- memory B
  never asked for, doesn't know about, but could still touch given a bug. Each load must
  explicitly revoke access to whatever extent exceeds *this* program's own needs, not just extend
  into more of it. The existing broad `tlbi vmalle1is` at the end of `load()` already covers the
  invalidate half of this for free (it was designed to be correct regardless of what changed, not
  a precise per-page flush).

**Demo:** load and run a deliberately oversized test binary (a static array well beyond the
previous 2 MiB ceiling, or Stage 16's `chrono-tz`-linked `date` itself) immediately followed by
`hello` -- confirming the large binary runs correctly, and that `hello`'s own (much smaller)
window is genuinely clean afterward, e.g. by having `hello` (or a dedicated test) attempt to read
memory beyond its own footprint and confirm it faults, proving the leftover extent was actually
revoked, not just left mapped and merely unused.

---

## Stage 18: growable memory at runtime -- `r18_brk`

**Goal:** let an already-running program ask for more memory as it goes, rather than only ever
getting a fixed allocation decided once at load time -- needed for anything whose memory needs
depend on runtime input, like Stage 13's editor opening a file of unknown size.

**Features:**
- A new syscall, `brk`-shaped: request the heap be extended to a new end address (or by some
  increment). The kernel handler reuses Stage 17's own mechanism -- map more of the same
  already-reserved, deliberately-free RAM immediately following whatever's already mapped for
  this program -- just triggered by an explicit runtime request instead of computed once from the
  ELF header.
- `userlib` gains its own `#[global_allocator]` -- reusing `linked_list_allocator`, the same crate
  the kernel's own heap already uses -- starting with a small initial heap and calling `brk` to
  grow it on demand, rather than syscalling on every individual allocation. `hello`/`crash` need
  none of this; the editor is the first EL0 program with genuine dynamic-allocation needs.
- The heap sits in the natural gap between the loaded segments (bottom of the window) and the
  stack (top, growing down) -- the classic Unix layout, already implied by this project's
  existing choice of where the stack lives. Worth a deliberate guard gap between wherever the
  heap has grown to and the stack, rather than assuming they'll never meet, matching this
  project's own established "leave it unmapped" philosophy elsewhere.
- **Audit item, not a feature:** once `alloc` is real in EL0, revisit every "assume this will fit"
  fixed-capacity workaround the no-heap constraint forced on earlier stages' user programs, and
  replace the ones that were only ever a stand-in for a real `Vec`/`String`. The concrete example
  already on record: Stage 12's `progs::PathBuf` (`user/progs/src/lib.rs`) joins a directory and a
  name into a fixed `PATH_MAX`-byte buffer and returns `None` on overflow, used by `mv`, `cp`'s
  directory-destination case, and `rm -r`/`chmod -R`'s recursion -- a `alloc::format!`-built
  `String` would need no such cap. Check for the same pattern elsewhere in `user/progs` before
  assuming this is the only instance.

**Demo:** open Stage 13's editor against progressively larger files -- confirming each one loads
and can be edited without hitting a fixed ceiling, and that the heap's growth is genuinely on
demand (a small file doesn't pre-allocate room for a large one it'll never open).

---

## Beyond Capstone 1, part 2: job control

The stages below form one coupled block, working toward a second capstone. Unix-style job control
needs *something* to background in the first place -- which means finally revisiting, not
incrementally patching around, the "at most one program is ever resident" assumption threaded
through Stage 9's single page table and Stage 12's single fd table. Deliberately scoped narrow
throughout: this block adds *at most two* simultaneously resident programs, never a general
N-process scheduler -- Capstone 2 itself (Stage 23) never needs more than that. The one exception is
Stage 24, added after the capstone specifically to prove something the capstone's own demos don't
need: genuine forced preemption between the two slots, not just cooperative handoff.

## Stage 19: two resident programs -- `r19_suspend`

**Goal:** the foundational primitive everything else in this block builds on -- letting a second
program stay alive, dormant, while the first one keeps running, instead of Stage 9's "exactly one,
and its state is abandoned the moment it stops" model. This is deliberately *not* a scheduler: it
adds the ability to suspend and later resume a program's exact state, not the ability to
time-slice between two actively-running ones.

**Features:**
- A second, independent user memory window, alongside the existing one -- Stage 17's per-load
  footprint computation applies to each window independently, so neither program pays for the
  other's size.
- A second, independent saved-EL0-context slot, generalizing `process.s`'s existing
  `enter_el0`/`resume_kernel` checkpoint mechanism. Today, `enter_el0` only ever checkpoints the
  *kernel's* own context (into `KERNEL_CTX`) so `resume_kernel` can return to it once a program
  exits or faults -- there's no way to checkpoint a *program's* own EL0-side state (`SPSR_EL1`/
  `ELR_EL1`/`SP_EL0`/`x0`-`x30`, all already sitting in the trap frame `kernel_entry` pushes for
  every syscall) instead of discarding it. This stage adds the symmetrical operation: a
  `suspend_current()` that saves the *currently running* program's state into a per-slot area
  rather than running it to completion, and a matching resume that restores it and `eret`s back in
  exactly where it left off.
- A second slot in `fd.rs`'s table, one per resident program rather than one shared table reset on
  every launch. Stage 12's own reasoning for a single table ("at most one program is ever
  resident") stops holding the moment a second one can be alive-but-dormant at the same time; each
  slot now keeps its own three-plus-`File(handle)` entries, still reset by `reset_for_launch()`
  when a *fresh* program is loaded into that slot, but no longer reset just because the *other*
  slot's occupant changed.
- **This stage is also the gate for moving the shell to userspace** (Stage 12 keeps it
  kernel-resident): a shell that launches a child has to stay loaded while the child runs, which needs
  exactly this second window and saved context. What remains after it is small and named in Stage 12's
  path-to-a-userspace-`sh` table: `spawn`/`wait` syscalls (Stage 22), turning Stage 12's shell-state frame
  into a per-process struct a child inherits (with the `chdir` syscall whose number Stage 12 reserves),
  and `dup2`-style fd control.
- Explicitly not a scheduler: switching between the two slots only ever happens at an explicit
  call from kernel code reacting to something specific (Stage 20's signal, Capstone 2's blocked
  pipe read/write) -- never a timer interrupt forcing a switch mid-instruction. No ready queue, no
  priority, no notion of "runnable."

**Demo:** a kernel-only harness (no shell/signal vocabulary exists yet to drive this from the
prompt): load a small test program into slot 0, let it run partway and print a marker, suspend it,
load and run a second test program in slot 1 to completion, then resume the first program from
slot 0 and confirm it continues exactly where it left off -- printing its own second marker
afterward, with any local state (e.g. a loop counter) intact. Expected output order proves the
interleaving actually happened: the first program's first marker, the second program's complete
output, then the first program's second marker.

---

## Stage 20: signals -- `r20_signals`

**Goal:** a kernel-to-program asynchronous notification mechanism, needed specifically for
`SIGTSTP` (`Ctrl+Z`) -- the first event in this project that's imposed on a still-running program
from outside, rather than something it calls voluntarily (`exit`) or synchronously traps into
(a segfault).

**Features:**
- `Ctrl+Z` recognized at the keyboard driver level (Stage 7; concretely, in Stage 12's token-queue
  producer, before the `Token` is ever queued), intercepted before it ever reaches
  whichever program currently owns keyboard input -- matching real termios' `ISIG` line-discipline
  behavior, where the terminal driver, not the foreground program, is what normally recognizes it.
- When recognized while a program occupies the foreground slot, the kernel calls Stage 19's
  `suspend_current()` on it directly. `SIGTSTP`'s default action (get suspended, nothing more)
  needs no program-side handler at all, so this first cut deliberately doesn't build general
  signal-handler registration (a `sigaction`-equivalent) -- narrow by design, the same spirit as
  Stage 9's segfault handling covering exactly the EC values it needs and nothing more.
- **A concrete, verified design precedent for how Stage 13's editor should behave once this
  exists**: real vim does *not* intercept `Ctrl+Z` -- it lets the terminal driver suspend it
  normally, the simpler and more common default. Real nano *does* intercept it (its own `SIGTSTP`
  handling), and has to provide `^T^Z` as an explicit escape hatch to actually suspend despite
  that. Stage 13's editor, vi-like by its own stated design reference, follows vim's precedent: it
  never reads `Ctrl+Z` as an editing keystroke, so this stage's kernel-level interception is the
  only thing that ever sees it, and no editor-side change is needed at all.
- **A second, closely-related signal, needed for correctness rather than authenticity: the
  `SIGTTIN` equivalent for background stdin.** Stage 7's keyboard driver only ever has one
  legitimate destination for "the current keystroke," so once Stage 19 lets a second program be
  resident in the background slot, that program's `Keyboard::read()` must not be allowed to
  silently consume input meant for whatever's actually in the foreground. If a background slot
  blocks on a keyboard read, the kernel suspends that slot on the spot (the same mechanism
  `Ctrl+Z` uses) instead of ever delivering it a keystroke -- resumed only once Stage 22's `fg`
  brings it back to the foreground. Unlike `SIGTSTP`, this isn't optional or deferrable: without
  it, a background job that happens to read stdin would race the shell for keystrokes.
- `SIGINT` (`Ctrl+C`, killing rather than suspending the foreground job) and `SIGCHLD` (notifying
  of a background job's exit) are the obvious next-most-needed signals, named here deliberately as
  *not* in scope -- this stage wires up exactly what Stage 22's job control needs to function, not
  a general signal subsystem.

**Demo:** none of its own -- `Ctrl+Z` has nothing useful to return control *to* until Stage 22's
shell vocabulary exists, so this stage is verified together with Stage 22's demo, below.

---

## Stage 21: sleep -- `r21_sleep`

**Goal:** a way for a program to voluntarily give up the CPU for a bounded duration, distinct from
every other way control has changed hands so far in this block (`Ctrl+Z`, an outside event; a
blocked pipe read/write, a consequence of what another program is doing). Introduced now
specifically so Stage 22's job-control demo has a genuinely useful long-running background program,
rather than an arbitrary busy-loop.

**Features:**
- A new syscall, `sleep`-shaped: takes a duration, returns once it's elapsed. Reuses Stage 19's
  `suspend_current()` directly -- sleeping *is* suspension, just with a deadline attached instead of
  an external trigger.
- A deadline field alongside each slot's saved context, checked from Stage 3's existing periodic
  timer IRQ handler (already firing regardless of which slot is in the foreground). On every tick,
  the handler checks whether any suspended slot's deadline has passed and, if so, resumes it. This
  is a *cooperative* wake, not preemption: nothing forces the resumed program to do anything in
  particular, it simply continues from wherever it called `sleep()`, which for a loop is typically
  straight into printing and calling `sleep()` again.
- Deadlines are computed from Stage 3's tick count (elapsed time), not Stage 14's RTC -- this is a
  scheduling primitive (a relative duration), not a calendar-time one; Stage 14's RTC stays reserved
  for `date`'s absolute wall-clock display.
- A `sleep` utility (a thin wrapper parsing a duration argument) and a small test program that loops
  `print; sleep(1s)` forever -- the concrete vehicle for Stage 22's background-job demo.
- **Worth naming for `jobs`'s sake (Stage 22)**: a sleeping background job and a `Ctrl+Z`-stopped
  one look identical at the slot level -- both "not running, has a saved context" -- distinguished
  only by *why* (a pending deadline vs. a delivered signal). `jobs` should report "Sleeping" rather
  than "Stopped" when that's the actual reason, even though the underlying suspend/resume mechanism
  is exactly the same either way.

**Demo:** a kernel-only harness, no shell needed yet: load the sleep-loop test program into slot 0;
while it's dormant waiting on its first deadline, load and run a second, short test program to
completion in slot 1; confirm the sleep-loop then resumes on its own on the next tick after its
deadline passes, without anything explicitly telling it to -- proving the wake is genuinely
deadline-driven, not something requiring an outside resume call the way Stage 19's own demo needed.

---

## Stage 22: job control in the shell -- `r22_jobs`

**Goal:** give Stage 12's shell the vocabulary for managing Stage 19/20/21's underlying mechanism --
`&`, `jobs`, `fg`, `bg` -- the same relationship Stage 16's `export` has to its environment stack:
the mechanism already exists, this stage is purely the shell-level interface to it.

**Features:**
- `cmd &`: the shell loads `cmd` into the background slot and returns to its own prompt
  immediately, instead of blocking until it exits.
- `jobs`: lists the background slot's occupant -- its command line and whether it's currently
  running or stopped.
- `fg`: swaps the background slot into the foreground slot (Stage 19's resume, now targeting
  slot 0) and hands it the keyboard again.
- `bg`: resumes a stopped or sleeping background job in place, without taking over the terminal --
  it keeps running (as long as it doesn't block on keyboard input) while the shell keeps its own
  prompt.
- `jobs` reports each job's actual state (running, stopped, or -- thanks to Stage 21 --
  sleeping), not just a generic "backgrounded."
- **Named limitation, not a bug**: at most one background job can exist at a time, a direct
  consequence of Stage 19's deliberate two-slot cap -- matching Stage 12's own precedent of naming
  a scope boundary explicitly (its "no infinite/streaming pipelines... ever") rather than leaving
  it implicit.
- **A deliberate decision on background stdout, matching real POSIX rather than adding new
  machinery to avoid it**: a background job's stdout still defaults to the shared `Console` fd
  (Stage 9's default, untouched unless explicitly redirected), and nothing suspends or buffers its
  writes -- the same behavior real terminals have by default (`TOSTOP` off), where a background
  job's output is simply allowed to interleave with whatever else is on screen. On a real Linux
  terminal running vim, this is exactly what happens when an unredirected background job writes
  output: it splices visually into vim's own display, purely cosmetically, and disappears the next
  time vim redraws from its own internal buffer. The same property holds here for free: Stage 13's
  editor already does a full-page rewrite from its in-memory buffer on every single edit, so any
  background-job corruption on screen is erased by the user's very next keystroke. A cosmetic wart,
  not a correctness issue -- no new machinery needed, and authentic to how real job control
  actually behaves.
- **The clean alternative, for anyone who doesn't want that wart**: the same escape hatch real
  Unix users reach for -- redirect the backgrounded command's stdout to a file (`cmd > log &`,
  already available from Stage 12) and poll that file instead of watching the shared console at
  all. This is where Stage 11's `tail` (print a file's last *N* lines, one-shot, no follow mode --
  already built there as a small variation on `cat`'s read loop) earns its keep: a concrete way to
  check a background job's progress by re-running `tail log` every so often without ever touching
  the framebuffer it's writing to.

**Demo:** launch Stage 21's `print; sleep(1s)` loop in the background with `&`; `jobs` shows it
sleeping/running; `fg` brings it to the foreground; `Ctrl+Z` stops it (now genuinely stopped,
distinct from merely sleeping); `bg` resumes it in the background; `jobs` reflects each state
change accurately throughout.

---

## Stage 23 (Capstone 2): job control and streaming pipes -- `r23_capstone2`

**Goal:** the second capstone, playing the same role for this block that Stage 13 played for
Stages 9-13 -- a demo that only works if every preceding stage in the block is genuinely correct,
combining job-controlling a real program (not a throwaway test binary) with the one limitation
Stage 12 itself named as permanent.

**Features:**
- **Real bounded-buffer, blocking pipes, replacing Stage 12's temp-file mechanism** (the pipeline
  code is isolated in one function there so it's replaceable; a kernel-heap buffer was considered for
  Stage 12 and not built -- still finite, so it wouldn't have lifted the limitation either). `cmd1 | cmd2`
  now loads both ends into the two resident slots at once (Stage 19), connected by a small
  fixed-size kernel buffer. Writing to a full buffer suspends the writer's slot and switches to the
  reader; reading an empty buffer symmetrically suspends the reader and switches to the writer --
  a targeted application of Stage 19's suspend/resume, triggered by blocked I/O rather than
  Stage 20's `Ctrl+Z` path. Still no timer-driven preemption anywhere in this: control changes
  hands only at these explicit blocking points, the same cooperative model Stage 19 established.
- This directly overturns Stage 12's own named-permanent limitation ("no infinite/streaming
  pipelines under this design, ever") -- `yes | head` becomes possible for the first time, since
  `yes` never has to finish producing (infinite) output before `head` starts consuming it.
- Stage 13's editor gets real job control with zero editor-side changes: since it never reads
  `Ctrl+Z` (Stage 20's vim precedent), suspending it, doing something else at the prompt, and
  `fg`-ing it back exercises the exact same mechanism already proven on throwaway test programs in
  Stages 19-22 -- now against a program with real state (an open file, cursor position, unsaved
  edits) that must survive the round trip correctly.

**Demo, two parts, mirroring Stage 13's own single end-to-end demo:**
1. **Job control:** open Stage 13's editor on a file, make an edit, `Ctrl+Z`, run a few other
   commands at the prompt (confirming the shell stayed fully responsive throughout), `fg` back in,
   confirm the edit and cursor position are exactly as left, save, and exit.
2. **Streaming pipes:** a `yes | head -n 5`-style pipeline produces exactly 5 lines and returns
   control to the prompt, despite `yes` itself never terminating on its own -- proof the pipe is
   genuinely streaming, not silently buffering all of `yes`'s (infinite) output before `head` ever
   gets to run.

---

## Stage 24: true preemptive multitasking -- `r24_preempt`

**Goal:** the one kind of context switch this whole block has deliberately avoided until now --
forced, not voluntary. Every switch built so far (`Ctrl+Z`, a blocked pipe, `sleep`) is either the
running program's own choice or a direct consequence of something it did; none of Capstone 2's own
demos need anything more, which is why this stage sits *after* it rather than inside it. This is
genuine single-core preemptive multitasking in the same sense real Unix has always used the term on
a uniprocessor -- one CPU, a periodic timer interrupt, the interrupt handler forcibly checkpointing
whatever's running and handing the CPU to something else, whether that program asked to give it up
or not. It's the real mechanism, not an approximation of it, just scoped here to two slots instead
of an arbitrary number. Worth being explicit about what it *isn't*: true parallelism (multiple
programs' instructions actually executing at the same instant) needs multiple physical cores, which
this project's single-core QEMU target doesn't have and doesn't need for this to be authentic --
single-core preemptive multitasking has never required more than one CPU, only a fast enough,
regular enough interrupt to make switching invisible.

**Features:**
- Stage 3's timer IRQ handler gains a new responsibility alongside Stage 21's deadline check: on
  every tick (or every Nth tick, a fixed quantum), it forcibly calls Stage 19's `suspend_current()`
  on whichever slot is presently running -- even if that program never called anything, never
  blocked, never slept -- and resumes the other slot. This is the second, and last, thing that
  needs the timer's IRQ to actually reach a running program -- and unlike what Stages 10-11 assumed,
  nothing has to be unmasked for it: since Stage 12 (Step 5) `run_program` no longer masks DAIF at EL0
  (the shell's loop runs outside IRQ context and the keyboard IRQ only enqueues `Token`s, so the
  reentrancy hazard the mask existed for is gone). What this stage adds is only that the timer's
  PPI is enabled at the GIC (as in Stage 3; the Stage 12 kernel doesn't use it yet) and its handler
  *acts* on the tick. Neither the keyboard's SPI nor `Blk`'s needs special handling: `Blk` only ever
  interrupts in response to something the kernel itself initiated and is already synchronously
  waiting on (`read_blocks_irq`/`write_blocks_irq`'s own `wfe` spin), never unsolicited.
- The quantum (ticks per turn) is a single fixed constant: strict round-robin between the (at most)
  two slots, no priority, no fairness accounting beyond that -- matching this whole block's
  established "as simple as correctly solving what's needed" scope.
- A slot that's genuinely dormant (sleeping, `Ctrl+Z`'d, blocked on pipe I/O) is skipped by the
  round-robin rather than force-resumed early: forced preemption only ever applies to a slot that's
  actually running and would otherwise keep the CPU indefinitely.

**Demo:** the one thing nothing earlier in this block could demonstrate, made quantifiable rather
than eyeballed. Two CPU-bound test programs, neither ever calling `sleep`, blocking on I/O, or
otherwise voluntarily yielding -- one writes an endless stream of `'a'`, the other an endless stream
of `'b'`, each via the ordinary `write(1, ...)` syscall -- loaded into both slots at once with no
shell interaction between them. `fd.rs`'s `Console::write` already routes through `ConsoleWriter`,
which unconditionally echoes to UART regardless of what's on the framebuffer, so the raw byte
stream is capturable straight from QEMU's UART-to-host-stdout passthrough (per this project's own
established capture technique -- background QEMU, redirect to a file, `kill`, then inspect it) --
sidestepping the framebuffer/`Console` corruption question entirely, not because it was fixed, but
because this demo never needs to look at the screen at all. Success is two-part, checked against
the captured byte stream: **finely interleaved** (short alternating runs of `a`s and `b`s, not one
long run of each in sequence -- which is exactly what Stage 19's own cooperative demo would produce
instead, its second program's *entire* output landing as one uninterrupted block), and **roughly
balanced** (close to a 50/50 split, confirming the fixed quantum is being applied evenly to both
slots, not starving one in favor of the other).
