//! Runs one EL0 program to completion (a normal `exit`, or a caught
//! fault) and returns to the caller like an ordinary function call. There
//! is no process table or scheduler here -- at most one program is ever
//! resident (see `arch/mmu.rs`'s own reasoning for why per-process machinery
//! has nothing to do in this system) -- but `run_program` still genuinely
//! *returns*, via the hand-rolled setjmp/longjmp in `arch/context.s`, rather
//! than requiring its caller to pass in "what happens next" as a
//! continuation.

use alloc::vec::Vec;

use aarch64_cpu::registers::{DAIF, ELR_EL1, SP_EL0, SPSR_EL1, Writeable};

use super::elfparse::ElfError;
use super::{argplan, elf};
use crate::platform::base_addresses::USER_STACK_TOP;
use crate::syscall::fd;
use abi::errno::{E2BIG, ENOEXEC};

unsafe extern "C" {
    /// Checkpoints this project's callee-saved register set and `eret`s
    /// into EL0 (`SPSR_EL1`/`ELR_EL1`/`SP_EL0` already set by
    /// `run`, below). Upholds the ordinary AAPCS64 calling
    /// convention exactly, so calling it from Rust needs no special
    /// handling -- see `arch/context.s`'s own doc comment for the full
    /// reasoning. Reached here only via `sym` (see `run`), never
    /// an ordinary Rust call, since `x0`-`x2` need to carry argc/argv/envp
    /// through to the `eret` untouched by anything Rust's own calling
    /// convention might otherwise do with them.
    fn enter_el0();

    /// Restores the register set `enter_el0` saved and jumps back into it
    /// directly, making `enter_el0` appear to return `code` -- the program's
    /// exit status, travelling in `x0` like a `longjmp` value. Called from
    /// `sync_el0_handler` (`syscall/mod.rs`) for `exit` and for a caught segfault
    /// alike; never returns itself.
    ///
    /// # Safety
    /// Must only be called from within the `sync_el0_64` handler, after
    /// `run` has actually started a program (so `arch/context.s`'s
    /// `KERNEL_CTX` holds a real checkpoint, not its zeroed initial
    /// state).
    pub fn resume_kernel(code: i32) -> !;
}

/// The status reported for a program stopped by a fault (`128 + SIGSEGV`, the shell convention),
/// since it never got to pass one to `exit` itself.
pub const EXIT_FAULT: i32 = 139;

/// The most stack a program's initial `argv` and `envp` together (strings plus pointer arrays) may
/// take -- far more than a typed line and a sensible environment can ever produce, and an eighth of
/// the 1 MiB stack, so an absurd list is refused up front instead of leaving the program almost no
/// stack of its own. Linux's `ARG_MAX` likewise counts both.
pub const ARG_MAX: usize = 128 * 1024;

/// Enumerates the possible reasons why a program couldn't be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// The file isn't a loadable executable (see `elfparse::ElfError` for exactly why).
    Elf(ElfError),
    /// The arguments and environment together don't fit in `ARG_MAX`.
    ArgsTooBig,
}

impl LaunchError {
    /// The `errno` this maps to -- what the launcher's message is worded from.
    pub fn errno(self) -> isize {
        match self {
            LaunchError::Elf(_) => ENOEXEC,
            LaunchError::ArgsTooBig => E2BIG,
        }
    }
}

impl From<ElfError> for LaunchError {
    fn from(e: ElfError) -> Self {
        LaunchError::Elf(e)
    }
}

/// A program that's loaded and ready to enter: everything `run` needs. Splitting load-and-set-up
/// (`prepare`) from entering (`run`) means a later stage that starts a program from inside another
/// (Stage 20's second resident slot) can reuse the first half unchanged.
pub struct PreparedProgram {
    /// The entry point of the program.
    entry: usize,
    /// The initial stack pointer of the program.
    sp: usize,
    /// The argument count.
    argc: usize,
    /// The argument vector (array of pointers to NUL-terminated strings).
    argv: usize,
    /// The environment, the same shape: `NAME=VALUE` strings.
    envp: usize,
}

/// Writes the argument and environment strings onto the stack below `sp` as NUL-terminated C
/// strings, plus a `NULL`-terminated array of pointers for each (see `argplan.rs` for the layout).
/// Returns `(argv, envp)`, the arrays' addresses; `argv` is also the new stack pointer. `None` if
/// the plan says everything wouldn't fit above `floor`.
///
/// SAFETY: `sp`/`floor` must lie inside the user window that `elf::load` just mapped, and nothing
/// may be running in it.
unsafe fn push_strings(sp: usize, floor: usize, args: &[&str], env: &[&str]) -> Option<(usize, usize)> {
    let plan = argplan::plan(sp, floor, &lens(args), &lens(env))?;

    for (item, &addr) in args.iter().chain(env).zip(plan.args.iter().chain(&plan.env)) {
        // SAFETY: `plan` keeps every string and both arrays within `floor..sp`.
        unsafe {
            core::ptr::copy_nonoverlapping(item.as_ptr(), addr as *mut u8, item.len());
            *((addr + item.len()) as *mut u8) = 0;
        }
    }

    // SAFETY: each array's `len + 1` slots were carved out of the window by `plan`; the last is the
    // terminating `NULL`.
    unsafe {
        for (array, addrs) in [(plan.argv, &plan.args), (plan.envp, &plan.env)] {
            let array = array as *mut usize;
            for (i, &addr) in addrs.iter().enumerate() {
                *array.add(i) = addr;
            }
            *array.add(addrs.len()) = 0;
        }
    }
    Some((plan.argv, plan.envp))
}

fn lens(items: &[&str]) -> Vec<usize> {
    items.iter().map(|s| s.len()).collect()
}

/// Loads `elf_bytes` and sets a fresh launch up: the ELF mapped into the user window, a reset file
/// descriptor table, and an initial stack with `args` as its `argv` and `env` (`NAME=VALUE` strings)
/// as its `envp`. Returns a `PreparedProgram` on success.
pub fn prepare(elf_bytes: &[u8], args: &[&str], env: &[&str]) -> Result<PreparedProgram, LaunchError> {
    // Set up the initial stack boundaries.
    let stack_top = USER_STACK_TOP;
    let floor = stack_top - ARG_MAX;

    // Dry run first: a list that can't fit must not cost a load.
    argplan::plan(stack_top, floor, &lens(args), &lens(env)).ok_or(LaunchError::ArgsTooBig)?;

    // Load the ELF binary into the user window.
    let entry = elf::load(elf_bytes)?;

    // Prepare the initial stack with the strings and the arrays of pointers.
    //
    // SAFETY: `stack_top` and `floor` are inside the user window `elf::load` just mapped; the
    // destination is otherwise-unused stack memory nothing touches until the program itself runs.
    let (argv, envp) = {
        let _user = crate::arch::mmu::user_access(); // writes the user stack, which PAN would forbid
        unsafe { push_strings(stack_top, floor, args, env) }.ok_or(LaunchError::ArgsTooBig)?
    };

    // Reset the file descriptor table for the new program -- last, so that nothing after it can fail:
    // the table now holds references to the shell's redirect files, and only `end_launch` (after the
    // program has run) lets go of them.
    fd::reset_for_launch();
    Ok(PreparedProgram {
        entry,
        sp: argv,
        argc: args.len(),
        argv,
        envp,
    })
}

/// Runs a `PreparedProgram` (from `prepare`) at EL0 to completion and returns its exit status -- `enter_el0`/
/// `resume_kernel` (`arch/context.s`) are what make an ordinary Rust function call correctly "pause"
/// for however long the EL0 program runs, however it ends.
///
/// The program runs with interrupts enabled, like any other code: a keyboard interrupt only feeds
/// the token queue (`keyboard/queue.rs`), so it can arrive at any moment without re-entering anything
/// -- the shell's loop, which called this, is not in an interrupt. (Until Step 5 it *was*, so every
/// DAIF bit had to stay masked at EL0.) Keys pressed while the program isn't reading wait in the queue
/// for the next reader, in order. Inside a syscall IRQs are masked, as on any exception entry; a
/// blocked `read(0)` fetches from the device itself (`stdin.rs`).
///
/// Interrupts are masked here from the first system-register write until the `eret`: an interrupt
/// in between would overwrite `ELR_EL1` and `SPSR_EL1`, and the `eret` would go somewhere else. The
/// `eret` itself unmasks them, by restoring the `SPSR_EL1` set below (whose DAIF bits are clear).
pub fn run(program: PreparedProgram) -> i32 {
    // Nothing may interrupt between here and the `eret` (see this function's doc comment).
    DAIF.write(DAIF::I::SET);

    // M[3:0] = bits 3:0 = 0b0000 (EL0t), DAIF bits 9:6 all clear: the program runs with interrupts on.
    SPSR_EL1.set(0);

    ELR_EL1.set(program.entry as u64); // Set the entry point for the EL0 program
    SP_EL0.set(program.sp as u64); // The write-cursor's final position -- see `prepare`.

    // SAFETY: entry/SP_EL0 above point into the user window the loader just mapped, for
    // whichever program is being run this call; x0/x1/x2 are the argc, argv and envp just written
    // there. Inline asm (rather than a plain `enter_el0()` call) specifically so x0-x2 are set
    // in the same instruction sequence that calls it, with nothing of Rust's own codegen
    // between the two free to reuse those (ordinarily caller-saved) registers first.
    //
    // On the way out, `x0` carries the program's exit status: `resume_kernel(code)` leaves it
    // there and `enter_el0`'s `ret` returns it, like any function's return value. Only the low 32
    // bits are meaningful (AAPCS64 says nothing about the upper half of a 32-bit value).
    let status: usize;
    unsafe {
        core::arch::asm!(
            "bl {enter}",
            inout("x0") program.argc => status,
            in("x1") program.argv,
            in("x2") program.envp,
            enter = sym enter_el0,
            clobber_abi("C"),
        );
    }

    // Back from the program (its `exit` or a fault, both taken as exceptions, so IRQs are masked
    // again): drop its fds, which closes every file only it held and so finishes any writes still in
    // progress, then unmask.
    fd::end_launch();

    // Ensure that interrupts are unmasked for the kernel after the program has finished,
    // even if the program crashed or exited abnormally.
    DAIF.write(DAIF::I::CLEAR);

    status as i32
}

/// Loads and runs `elf_bytes` with `args` as its `argv` and `env` as its `envp`, returning its exit
/// status -- `prepare` then `run`. `Err` if it couldn't be started at all (nothing ran).
pub fn run_program(elf_bytes: &[u8], args: &[&str], env: &[&str]) -> Result<i32, LaunchError> {
    Ok(run(prepare(elf_bytes, args, env)?))
}
