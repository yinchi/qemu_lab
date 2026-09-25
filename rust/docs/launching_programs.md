# Launching programs

How the shell starts an EL0 program: finding and loading it, entering and leaving EL0, and passing
it its arguments. Where the program's memory comes from -- the page tables and the fixed user window
it is mapped into -- is described in [`mmu.md`](mmu.md); this document only relies on that window
existing. Who calls `launch`, and what a command line has to go through first, is in
[`shell.md`](shell.md); the programs themselves are listed in [`progs.md`](progs.md), and how a program's
`read(0)` gets its lines is in [`console.md`](console.md). The system calls a running program makes are in
[`syscalls.md`](syscalls.md), and the files it opens in [`filesystem.md`](filesystem.md).

## How programs run

The entry point for launching a userspace program from the kernel is `launch` in
`shell/launch.rs`:

```rust
pub fn launch(vol: &FatVolume<BlkIo>, argv: &[&str], depth: usize) -> Option<i32>
```

This finds the program named by `argv[0]`, ensuring it exists, is a file, and has its executable
attribute bit (custom-defined using an unused bit in the FAT specification) set, then reads it into
memory and invokes `process::run_program`. Problems are reported through the shell's own error path
(`shell_err`, so they are redirectable like any other shell message). It returns `None` if no program
actually ran, or `Some(code)` with its exit status if one did &mdash; whether to print a nonzero
status is the caller's decision, not `launch`'s. `depth` is the script-nesting depth, used only by
the fallback below.

An executable file that doesn't start with the ELF magic is bash's `ENOEXEC` fallback rather than an
error: if its first 128 bytes contain no NUL it "looks like" a script and is run as one (via
`shell::run_script_content`, hence the `depth` parameter), otherwise it is refused as binary garbage.

> [!NOTE]
> The volume is mounted as `/` (root). A command word containing a `/` is a path (relative to the
> shell's working directory unless it starts with `/`); a bare name is looked up in `/bin` only &mdash;
> as `name`, then `name.exe` &mdash; independent of the working directory, with no `PATH`-style search.

### `process::run_program`

`run_program(elf_bytes: &[u8], args: &[&str]) -> Result<i32, LaunchError>` (`exec/process.rs`) does
everything from "here's an ELF file and its arguments" to "that program ran to completion," as two
halves &mdash; `prepare` (load and set up) then `run` (enter EL0) &mdash; split so a later stage
that starts a program from inside another can reuse the first half unchanged. It returns the
program's exit status, or a `LaunchError` (`Elf(ElfError)` for a file that isn't a loadable
executable, `ArgsTooBig`) if nothing could be started.

`prepare(elf_bytes, args, env) -> Result<PreparedProgram, LaunchError>`, where `env` is the program's environment as `NAME=VALUE` strings (`launch` builds it from the shell's exported variables):

1. Plans the argument and environment layout as a dry run against `ARG_MAX` (128 KiB of stack) first, so a
   list that can't fit is refused before it costs a load.
2. `elf::load(elf_bytes)` maps the ELF's `PT_LOAD` segments into the user window (only the pages they and the stack need) and
   returns its entry point.
3. `fd::reset_for_launch()` gives the new program a fresh file descriptor table (its standard
   streams bound to whatever the shell's current frame says).
4. Writes `args` and `env` onto the program's stack as a C-style `argc`/`argv` and `envp` (see "Passing arguments to
   userspace programs" below for the full mechanism), producing the address that becomes the
   program's initial `SP_EL0`. This holds a `mmu::user_access()` guard, since PAN would otherwise
   forbid the kernel writing the user stack (see "Turning translation on" in [`mmu.md`](mmu.md)).

`run(program: PreparedProgram) -> i32`:

1. Masks IRQs from the first system-register write until the `eret`: an interrupt in between would
   overwrite `ELR_EL1`/`SPSR_EL1` and the `eret` would go somewhere else.
2. Sets `SPSR_EL1` to 0: EL0t with every DAIF bit *clear*, so the program runs **with interrupts
   enabled**. The `eret` itself is what unmasks them.
3. Sets `ELR_EL1` (entry point) and `SP_EL0` (the address from `prepare`), then loads `argc`/`argv`/`envp`
   into `x0`/`x1`/`x2` in the *same* inline-asm block that calls `enter_el0` -- specifically so
   nothing of Rust's own codegen can reuse those registers first.
4. Once `enter_el0` "returns" (see below for what that actually means), `x0` holds the program's exit
   status, `fd::end_launch()` closes every file the program left open (finishing any writes still in
   progress), and `DAIF.I` is cleared again. `resume_kernel`'s jump back here is a raw branch, not an
   `eret`, so it never restores `DAIF` the way returning from an exception normally would -- without
   this, the shell would go deaf to the keyboard after the very first program it ever ran.

From the caller's point of view, `run_program` is an ordinary function call that happens to take
a long time -- it doesn't matter to `launch` whether the program exited cleanly or crashed (a fault
is reported as exit status 139, `128 + SIGSEGV`, the shell convention).

> [!NOTE]
> A keyboard interrupt only feeds the token queue (`keyboard/queue.rs`); the shell's loop, which
> called `run_program`, is ordinary code and not itself in an interrupt, so an IRQ arriving while a
> program runs re-enters nothing. Keys pressed while the program isn't reading wait in the queue,
> in order, for the next reader. Inside a syscall IRQs are masked, as on any exception entry, so a
> blocked `read(0)` fetches from the device itself (`keyboard/stdin.rs`). Until Stage 12's Step 5
> the shell's own loop *was* inside the keyboard interrupt handler, which is why every DAIF bit used
> to have to stay masked while a program ran: a nested IRQ would have re-entered the handler,
> remapped the user window the running program was executing out of, and overwritten the
> single-slot `KERNEL_CTX` checkpoint (below).

### `arch/context.s`

`arch/context.s` is a hand-rolled `setjmp`/`longjmp` pair, since there's no process table or
scheduler to save a "current program" state in -- `run_program` needs to look like it's just
pausing for however long the EL0 program runs, then continuing exactly where it left off, however
that program ends.

`KERNEL_CTX` is 14 `.bss` slots (112 bytes): `sp`, a resume address, and the callee-saved
registers `x19`-`x30` (AAPCS64 only requires saving these across a call -- caller-saved
registers don't need to survive a plain `bl`).

- **`enter_el0`** (the "setjmp" half): saves the current `sp`, the address of its own local label
  right after the `eret`, and `x19`-`x30` into `KERNEL_CTX`, then `eret`s into EL0
  (`SPSR_EL1`/`ELR_EL1`/`SP_EL0` are already set by `run` before this is called). Because
  it saves and restores exactly what AAPCS64 already requires around any ordinary call, calling
  it from Rust needs no special handling at all.
- **`resume_kernel`** (the "longjmp" half): restores `sp` and `x19`-`x30` from `KERNEL_CTX`, then
  branches *directly* to the saved resume address -- not a `ret`, since this isn't returning from
  a call, it's jumping back into the middle of `enter_el0`, which is (from the CPU's perspective)
  still mid-execution, waiting at that label. Once there, `enter_el0` does an entirely ordinary
  `ret`, using the `x30` that was live when it was first called -- which is what makes
  `run` see this as a normal return. `resume_kernel` takes the exit status as its argument and
  leaves it in `x0`, so it reaches `enter_el0`'s `ret` as the return value &mdash; a `longjmp`
  value, in `setjmp`/`longjmp` terms. Called from `sync_el0_handler` (`syscall/mod.rs`) for a
  genuine `exit` syscall or a caught segfault alike; never returns itself.

## Passing arguments to userspace programs

The launcher tokenizes a typed line into a program name plus arguments and hands the whole thing
to the new program as `argc`/`argv`, the same convention a real Unix `exec` uses. Getting a
`Vec<String>` on the kernel side into something a `main(argc, argv)` on the userspace side can
actually read means crossing the `eret` into EL0 partway through, so the representation changes
shape more than once along the way.

### Building the stack layout (kernel side)

The typed line is split by the shell's own lexer and parser (`shell/lexer.rs`, `shell/syntax.rs`;
see `shell.ebnf` for the grammar) into pipeline stages, each with its words -- quoting and backslash
escapes handled, not just whitespace splitting -- and its redirections. A stage's words become a
`Vec<String>` with the program name in slot 0, which `launch` receives as a `&[&str]`.

`process::prepare` is where that `&[&str]` actually becomes memory a userspace program can read.
The layout itself is worked out by a separate pure module, `exec/argplan.rs` (`argplan::plan`, host
tested), which never touches memory; `process::push_strings` then writes what it planned. Starting
from `USER_STACK_TOP` and working *downward* (the direction a stack grows), each argument's raw UTF-8
bytes are laid out followed by a NUL terminator -- a NUL is needed here specifically because nothing
else carries a length across the `eret` boundary that's coming up. Each string's resulting address is
recorded as it goes. Once every string is placed, a second region just below them holds the pointer
array itself: one `usize` slot per recorded address plus a final `NULL` terminator (so `argv[argc]`
is `NULL`, as in C), 16-byte-aligned. That array's own base address becomes the program's initial
stack pointer (`SP_EL0`).

The environment (Stage 17) goes the same way: its `NAME=VALUE` strings are placed below the arguments'
strings, and the one block of pointers holds `argv[]` and its `NULL`, then `envp[]` and its `NULL`, so `envp`
is `argv + argc + 1` slots, as on Linux. Only the block's base is aligned, and it is `argv`.

The whole layout, arguments and environment together (as with Linux's `ARG_MAX`), must fit within `ARG_MAX`
(128 KiB, an eighth of the 1 MiB stack) above `USER_STACK_TOP - ARG_MAX`; a longer list is refused up front as `LaunchError::ArgsTooBig`
(reported as "Argument list too long"), so an absurd one can't leave the program almost no stack of
its own. The plan is checked as a dry run *before* the ELF is loaded, so a refusal costs nothing.

`argc`, `argv` and `envp` are then loaded into `x0`/`x1`/`x2` in the same inline-asm
block that calls `enter_el0` (`arch/context.s`), specifically so nothing in between can reuse those
registers first. Neither `enter_el0` nor the `eret` inside it touch them, and neither does
`userlib`'s `_start` before its own `bl main` -- so the exact register values `run` set
survive, untouched, all the way to the new program's entry point.

### Decoding argv and envp (userspace side)

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
it without any special handling -- `echo` (`user/progs/src/bin/echo.rs`), the first program built
against `entry_with_args!`, drops its own name with a plain `args.skip(1)` before printing the
rest back.

`envp` is read the same way, one step removed: a program started with `userlib::entry_with_env!` (whose
generated `main` takes `x2` as a third argument) has the pointer stored in a static by `userlib::env`, and
`env::var(name)` and `env::vars()` scan it on demand, splitting each `NAME=VALUE` string at its first `=`. A program
started with `entry!` or `entry_with_args!` never records it and sees an empty environment; a binary built for a
kernel older than Stage 17 (which leaves `x2` as whatever it was) is never affected, since nothing there calls `env`.
The programs that use it are `env` and `printenv` (`user/progs_r17/`).
