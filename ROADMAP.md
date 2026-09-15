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
  "echo line" demo already proved, stopping at Enter. Deliberately not Stage
  5's cursor-aware editor: this stage is a hard prerequisite for testing
  utilities, not the polished interactive experience Stage 12 aims for.
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
- Extends Stage 9's minimal `write`/`read`/`exit` syscall set to
  `open`/`read`/`write`/`close` -- needed by everything here (`echo` already
  exists from Stage 10, built there specifically to prove `argc`/`argv`
  setup, and needed none of these). `ls` additionally needs some way to
  enumerate directory entries -- a dedicated `readdir`-style syscall, or
  treating a directory as a readable pseudo-file of raw entry records, are
  both reasonable; worth deciding when this stage is actually reached rather
  than committing now.
- `cat`, `ls`, `cp` as real, independent programs -- each one loaded through
  Stage 9's ELF loader like any other, not special-cased, and each one a
  real consumer of the `argc`/`argv` mechanism Stage 10 built for `echo`.
- **An open design question worth naming rather than silently deciding**:
  separate small binaries (simpler, each one a clean standalone exercise of
  the loader) vs. one true multi-call "busybox" binary that dispatches on how
  it was invoked (`argv[0]`) -- authentic to the name, but needs the shell
  and/or filesystem to support multiple names resolving to the same file
  (traditionally via symlinks), which is new surface area of its own.

**Demo:** run each new utility via Stage 10's minimal launcher: `cat` on a
known file prints its contents; `ls` lists the root directory read via
Stage 8's filesystem; `cp` copies a file and the copy's contents are
confirmed to match -- alongside `echo`, already working since Stage 10.

---

## Stage 12: a real shell, with pipes -- `r12_shell`

**Goal:** upgrade Stage 10's bare launcher into something worth typing at
regularly, now that Stage 11 gives it real programs worth combining -- not a
rebuild from scratch, an extension of the same tokenizer and ELF-loader
invocation Stage 10 already established.

**Features:**
- Real line editing with history, reusing Stage 5's cursor-aware buffer
  (insert/remove at an arbitrary position) directly -- but no longer reusing
  Stage 5's CSI parser or its ANSI reprint-the-tail redraw: input now comes
  from Stage 7's `virtio-keyboard` as discrete, unambiguous key events (no
  escape sequences to parse, and hence nothing for `noline`'s unbounded-ESC
  problem to even apply to), and redraws go straight to Stage 6's console --
  there's no remote cursor to reposition blindly, so the cell that changed
  is simply rewritten directly via `put_char`. Making the cursor itself
  visible needs its own small solution, though: unlike VGA's hardware cursor
  register, `virtio-gpu` has no cell-aware cursor at all (only a 64x64 ARGB
  pointer overlay, sized for a mouse pointer, not a character cell), so a
  visible text cursor is drawn as an ordinary glyph -- e.g. a solid block --
  via the same `put_char` path as any other character. History is a thin
  addition on top: a ring buffer of past lines,
  with Up/Down just two more keycodes Stage 7's driver already delivers as
  discrete events, no new parsing needed.
- Redirection (`>`/`<`): close to free, given Stage 11's `open`/`read`/
  `write`/`close` syscalls already exist -- `cmd > file` is just "open the
  file, hand its descriptor to `cmd` as stdout."
- **Pipes (`cmd1 | cmd2`), via temp files -- deliberately not true streaming
  concurrency.** Real pipe semantics need two processes actually running at
  once, which Stage 9 explicitly rules out. Early MS-DOS hit this identical
  single-tasking wall and solved it the same way we will: run `cmd1` to
  completion with stdout redirected to a temp file on Stage 8's filesystem,
  then run `cmd2` to completion with stdin redirected from that file, then
  delete it. **Named limitation, not a bug to fix later:** this only works
  for pipelines whose stages produce a *finite* amount of output that fits on
  disk -- no infinite/streaming pipelines under this design, ever.

**Demo:** a `> ` prompt on Stage 6's display with working cursor movement
and command history via Stage 7's keyboard; `echo hello | cat` and
`ls > listing.txt` both work via the temp-file mechanism above, using
Stage 11's real utilities rather than synthetic test programs; control
returns to the prompt once each stage exits (via the `exit` syscall from
Stage 9).

---

## Stage 13 (capstone): a vi-like full-screen editor -- `r13_editor`

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

**Demo:** launch the editor from Stage 12's shell against a file already
present on the disk image, edit its text on Stage 6's display using
Stage 7's keyboard, save it, then -- to prove persistence, not just an
in-memory illusion -- restart QEMU against the same `disk.img` and confirm
the edit is still there.
