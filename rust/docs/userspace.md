# Userspace, the MMU, and running programs

## The Memory Management Unit (MMU)

The MMU maps virtual addresses to physical addresses, controlling access permissions and memory
attributes for different regions. In this project, it sets up page tables for the kernel and a
fixed user window, ensuring that the kernel and user programs have the correct memory access rights.

For simplicity, this project, as of Stage 9, uses a single shared **identity-mapped** page table
for the kernel and a fixed user window, avoiding the complexity of per-process page tables and
ASIDs (Address Space Identifiers).  Identity mapping means that virtual addresses are the same as
physical addresses, simplifying address translation and access control.

The memory layout is defined in `mmu::enable`, where the kernel and user regions are mapped with
appropriate attributes.  Since there is only one shared page table (between the kernel and one user
program at a time), there is a single `aarch64_paging::IdMap` instance that governs all address
translations &mdash; this is assigned an ASID of 0.

We also define the root level of the page table hierarchy as 1, which creates a three-level page
table structure (4 KiB level-3 pages, 2MiB level-2 blocks, and 1 GiB level-1 blocks, with 512
entries per table meaning the level-1 table can cover 512 GiB of virtual address space).  Finally,
the regime of the `IdMap` is set to `El1And0`, meaning it governs address translations for both EL1
(kernel) and EL0 (user) levels, the only two levels our `virt` QEMU setup uses.

```rust
use aarch64_paging::{idmap::IdMap, paging::El1And0};

const ROOT_LEVEL: usize = 1;
const ASID: usize = 0; // Only ever one address space -- no per-process ASIDs needed.

let mut idmap = IdMap::with_asid(ASID, ROOT_LEVEL, El1And0);
```

### Mapped region attributes

`aarch64_paging::descriptor::El1Attributes` defines the following memory attributes for page table entries:

- `ATTRIBUTE_INDEX_0`: Read attribute bits from slot 0 in the MAIR_EL1 register.
- `ATTRIBUTE_INDEX_1`: Read attribute bits from slot 1 in the MAIR_EL1 register.
- `VALID`: Marks the entry as valid.
- `ACCESSED`: Marks the entry as accessed.
- `UXN`: Unprivileged Execute Never -- prevents execution at EL0.
- `PXN`: Privileged Execute Never -- prevents execution at EL1.
- `INNER_SHAREABLE`: Marks the memory as coherent across every observer in the same inner shareable
domain -- every CPU in the same coherent cluster, plus any DMA-capable agent considered part of
that same interconnect.

  - A VirtIO device's *register window* (the 0x200-byte `virtio-mmio` slot the CPU pokes to
  configure it) is *not* marked as inner sharable;
  - The data it *actually DMAs to/from* (`virtio_hal.rs`'s `DMA_POOL`, virtqueues, a framebuffer,
  disk blocks) is ordinary RAM, and is inner shareable.

- `READ_ONLY`: Marks the memory as read-only.

Our `MAIR_EL1` register is configured with `MAIR_NORMAL` in slot 1 and `MAIR_DEVICE_NGNRE` in slot 0:

```rust
const ATTR_DEVICE_INDEX: u64 = 0;
const ATTR_NORMAL_INDEX: u64 = 1;

const MAIR_DEVICE_NGNRE: u64 = 0b0000_0100;
const MAIR_NORMAL: u64 = 0xff;

MAIR_EL1.set(
    (MAIR_DEVICE_NGNRE << (8 * ATTR_DEVICE_INDEX)) | (MAIR_NORMAL << (8 * ATTR_NORMAL_INDEX)),
);
```

Each 8-bit slot in the MAIR_EL1 register is further broken down as follows:

- If bits `[7:4]` are `0000`, the whole byte encodes a **Device** memory type, and bits `[3:2]`
  select which one: `00` = nGnRnE, `01` = nGnRE, `10` = nGRE, `11` = GRE. Bits `[1:0]` are unused.

  - **Gathering (`G`)**: whether separate accesses to the same address may be merged into fewer,
  larger bus transactions.
  - **Re-ordering (`R`)**: whether accesses to the device may complete out of program order.
  - **Early Write Acknowledgement (`E`)**: whether a write can be reported "done" before it has
  actually reached the device.

- Otherwise, the byte encodes **Normal** memory, and splits into two independent 4-bit fields:
bits `[7:4]` for the *Outer* cacheability policy, bits `[3:0]` for the *Inner* one. Each nibble's
own encoding includes `0100` = Non-cacheable, and `11WR` = Write-Back Non-transient, where `W`/`R`
are independent Write-Allocate/Read-Allocate hints.
  - **Inner**: refers to cache levels closest to the processor core, e.g. L1.
  - **Outer**: refers to cache levels further away from the processor core.
  - **Write-back**: data is written to the cache first and later propagated to the next level of
  memory.
  - **Non-transient**: the memory is expected to remain valid for a longer period and not be
  frequently evicted from the cache.
  - **Write-Allocate (`W`)**: whether a write miss should allocate a new cache line.
  - **Read-Allocate (`R`)**: whether a read miss should allocate a new cache line.

Working through this project's own two constants:

- `MAIR_DEVICE_NGNRE = 0b0000_0100`:
  - bits `[7:4]` = `0000` (Device)
  - bits `[3:2]` = `01` (nGnRE)
- `MAIR_NORMAL = 0xff = 0b1111_1111`: both the Outer (`[7:4]`) and Inner (`[3:0]`) nibbles are
`1111` &mdash; Write-Back Non-transient with both Write-Allocate and Read-Allocate set &mdash;
matching `mmu.rs`'s own comment on the constant.

### Attribute groups

The following attribute groups are defined in `mmu.rs` for memory regions in this project:

- Device: `ATTRIBUTE_INDEX_0 | VALID | ACCESSED | UXN | PXN`
- Kernel, read-execute: `ATTRIBUTE_INDEX_1 | INNER_SHAREABLE | VALID | ACCESSED | READ_ONLY | UXN`
- Kernel, read-only: `ATTRIBUTE_INDEX_1 | INNER_SHAREABLE | VALID | ACCESSED | READ_ONLY | UXN | PXN`
- Kernel, read-write: `ATTRIBUTE_INDEX_1 | INNER_SHAREABLE | VALID | ACCESSED | UXN | PXN`

where Attribute Indexes 0 and 1 correspond to `MAIR_DEVICE_NGNRE` and `MAIR_NORMAL`, respectively.

> [!NOTE]
> `elf.rs` can create memory regions with UXN unset and PXN set, for user-level-only code
> execution &mdash; the inverse of `kernel_rx` above (UXN set, PXN unset), which is EL1-only.

### Initial memory setup

Upon initialization, before any user program is loaded, the following memory regions are set up
according to `mmu.rs`:

- General interrupt controllor distributor (GICD): `device` attribute group
- General interrupt controllor CPU interface (GICC): `device` attribute group
- UART device memory-mapped registers: `device` attribute group
- VirtIO device memory-mapped registers: `device` attribute group

All of the above memory regions fit well within 1 GiB of virtual address space. Then, from the 1GiB
mark (0x4000_0000) onward, the kernel image sections are mapped with their respective attribute
groups.

- Kernel image, `.text` section: `kernel_rx` attribute group
- Kernel image, `.rodata` section: `kernel_ro` attribute group
- Kernel image, `.data` and `.bss` sections and stack: `kernel_rw` attribute group

Finally, user space starts at 0x4400_0000 as defined in `base_addresses.rs`, meaning the kernel is
assigned 64 MiB of virtual address space; but no memory regions are mapped there until a user
program is loaded.

## How programs run

The entry point for launching a userspace program from the kernel is `launch` in `main.rs` (as of
Stage 10):

```rust
fn launch(
    vol: &hadris_fat::sync::FatVolume<BlkIo>,
    argv: &Argv,
    uart: &mut UartWriter,
    console: &mut Console,
) {
    // ...
}
```

This checks the volume for the given program in `argv[0]`, ensuring it exists and its executable
attribute bit (custom-defined using an unused bit in the FAT specification) is set.  If these
conditions are met, the program is loaded into memory and invoked via `process::run_program`.

> [!NOTE]
> The volume is assumed to be mounted as `/` (root) and binaries as of Stage 10 are expected to be
> located in `/bin`.

### `process::run_program`

`run_program(elf_bytes: &[u8], args: &[&str])` (`process.rs`) does everything from "here's an ELF
file and its arguments" to "that program ran to completion," in five steps:

1. `elf::load(elf_bytes)` maps the ELF's `PT_LOAD` segments into the fixed user window and
   returns its entry point.
2. Writes `args` onto the program's stack as a C-style `argc`/`argv` (see "Passing arguments to
   userspace programs" below for the full mechanism), producing the address that becomes the
   program's initial `SP_EL0`.
3. Sets `SPSR_EL1` with every DAIF bit *masked*, not cleared -- no interrupt of any kind reaches
   the program while it runs at EL0. This closes a real re-entrancy hazard: a keyboard IRQ landing
   mid-program would otherwise re-enter `handle_keyboard_irq` while this very call is still on
   the stack, and a second `run_program` from that nested call would remap the same user window
   the first program is currently executing out of, and stomp the single-slot `KERNEL_CTX`
   checkpoint (below) its own `enter_el0` just wrote.
4. Sets `ELR_EL1` (entry point) and `SP_EL0` (the address from step 2), then loads `argc`/`argv`
   into `x0`/`x1` in the *same* inline-asm block that calls `enter_el0` -- specifically so
   nothing of Rust's own codegen can reuse those registers first.
5. Once `enter_el0` "returns" (see below for what that actually means), re-clears `DAIF.I`.
   `resume_kernel`'s jump back here is a raw branch, not an `eret`, so it never restores `DAIF`
   the way returning from an exception normally would -- without this, the shell would go deaf
   to the keyboard after the very first program it ever ran.

From the caller's point of view, `run_program` is an ordinary function call that happens to take
a long time -- it doesn't matter to `launch` whether the program exited cleanly or crashed.

> [!NOTE]
> Programs that need to be interruptable (raw-mode input, scheduling interrupts) are planned for
> future stages &mdash; as of Stage 10, all programs run with DAIF fully masked.

### `process.s`

`process.s` is a hand-rolled `setjmp`/`longjmp` pair, since there's no process table or scheduler
to save a "current program" state in -- `run_program` needs to look like it's just pausing for
however long the EL0 program runs, then continuing exactly where it left off, however that
program ends.

`KERNEL_CTX` is 14 `.bss` slots (112 bytes): `sp`, a resume address, and the callee-saved
registers `x19`-`x30` (AAPCS64 only requires saving these across a call -- caller-saved
registers don't need to survive a plain `bl`).

- **`enter_el0`** (the "setjmp" half): saves the current `sp`, the address of its own local label
  right after the `eret`, and `x19`-`x30` into `KERNEL_CTX`, then `eret`s into EL0
  (`SPSR_EL1`/`ELR_EL1`/`SP_EL0` are already set by `run_program` before this is called). Because
  it saves and restores exactly what AAPCS64 already requires around any ordinary call, calling
  it from Rust needs no special handling at all.
- **`resume_kernel`** (the "longjmp" half): restores `sp` and `x19`-`x30` from `KERNEL_CTX`, then
  branches *directly* to the saved resume address -- not a `ret`, since this isn't returning from
  a call, it's jumping back into the middle of `enter_el0`, which is (from the CPU's perspective)
  still mid-execution, waiting at that label. Once there, `enter_el0` does an entirely ordinary
  `ret`, using the `x30` that was live when it was first called -- which is what makes
  `run_program` see this as a normal return. Called from `sync_el0_handler` (`syscall.rs`) for a
  genuine `exit` syscall or a caught segfault alike; never returns itself.

## Passing arguments to userspace programs

Starting from `r10_repl/`, the launcher tokenizes a typed line into a program name plus
arguments and hands the whole thing to the new program as `argc`/`argv`, the same convention a
real Unix `exec` uses. Getting a `Vec<String>` on the kernel side into something a
`main(argc, argv)` on the userspace side can actually read means crossing the `eret` into EL0 
partway through, so the representation changes shape more than once along the way.

### Building the stack layout (kernel side)

`argv::Argv::parse()` splits the finished line via the `shlex` crate (real POSIX shell-word
syntax -- quoting, backslash escapes -- not just whitespace splitting), producing a `Vec<String>`
with the program name in slot 0. `Argv::as_argv()` turns that into a `Vec<&str>`, borrowing each
string rather than copying it.

`process::run_program` is where that `&[&str]` actually becomes memory a userspace program can
read. Starting from the top of the program's fixed stack window and working *downward* (the
direction a stack grows), it writes each argument's raw UTF-8 bytes followed by a NUL
terminator -- a NUL is needed here specifically because nothing else carries a length across the
`eret` boundary that's coming up. Each string's resulting address is recorded as it goes. Once
every string is written, a second region just below them holds the pointer array itself: `argc`
slots of `usize`, one per recorded address, 16-byte-aligned. That array's own base address
becomes the program's initial stack pointer (`SP_EL0`).

`argc` and the pointer array's address are then loaded into `x0`/`x1` in the same inline-asm
block that calls `enter_el0` (`process.s`), specifically so nothing in between can reuse those
registers first. Neither `enter_el0` nor the `eret` inside it touch `x0`/`x1`, and neither does
`userlib`'s `_start` before its own `bl main` -- so the exact register values `run_program` set
survive, untouched, all the way to the new program's entry point.

### Decoding argv (userspace side)

On the far side of `eret`, `main` receives those same two register values as `argc: usize` and
`argv: *const *const u8` -- a type reinterpretation, not a conversion: the kernel wrote plain
address-sized integers, and the userspace side just reads the same bits as a pointer type
instead, valid since both are 8 bytes on AArch64. A program written with `userlib::entry!` never
sees these at all; one written with `userlib::entry_with_args!` has them decoded automatically
into a `userlib::Args`:

```rust
pub struct Args {
    argv: *const *const u8,
    remaining: usize,
}
```

Constructing an `Args` (via `userlib::args()`) doesn't decode anything yet -- it just stores the
base pointer and the count. The actual work happens lazily, one argument at a time, in `Args`'s
`Iterator` implementation: each call to `next()` dereferences the current slot of the pointer
array to get one argument's address, scans forward byte-by-byte for the NUL terminator the
kernel wrote, builds a `&[u8]` slice of that length, advances to the next slot, and only then
validates the bytes as UTF-8 to produce the `&'static str` it actually returns.

Because `Args` is a plain `Copy` iterator, a consumer can use ordinary `Iterator` combinators on
it without any special handling -- `echo` (`user/echo/src/main.rs`), the first program built
against `entry_with_args!`, drops its own name with a plain `args.skip(1)` before printing the
rest back.
