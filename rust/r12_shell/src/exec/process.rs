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
    /// `run_program`, below). Upholds the ordinary AAPCS64 calling
    /// convention exactly, so calling it from Rust needs no special
    /// handling -- see `arch/context.s`'s own doc comment for the full
    /// reasoning. Reached here only via `sym` (see `run_program`), never
    /// an ordinary Rust call, since `x0`/`x1` need to carry argc/argv
    /// through to the `eret` untouched by anything Rust's own calling
    /// convention might otherwise do with them.
    fn enter_el0();

    /// Restores the register set `enter_el0` saved and jumps back into it
    /// directly, making `enter_el0` appear to return `code` -- the program's
    /// exit status, travelling in `x0` like a `longjmp` value. Called from
    /// `sync_el0_handler` (`syscall.rs`) for `exit` and for a caught segfault
    /// alike; never returns itself.
    ///
    /// # Safety
    /// Must only be called from within the `sync_el0_64` handler, after
    /// `run_program` has actually started a program (so `arch/context.s`'s
    /// `KERNEL_CTX` holds a real checkpoint, not its zeroed initial
    /// state).
    pub fn resume_kernel(code: i32) -> !;
}

/// The status reported for a program stopped by a fault (`128 + SIGSEGV`, the shell convention),
/// since it never got to pass one to `exit` itself.
pub const EXIT_FAULT: i32 = 139;

/// The most stack a program's initial `argv` (strings plus pointer array) may take -- far more than
/// a typed line can ever produce, and an eighth of the 1 MiB stack, so an absurd argument list is
/// refused up front instead of leaving the program almost no stack of its own.
pub const ARG_MAX: usize = 128 * 1024;

/// Enumerates the possible reasons why a program couldn't be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    /// The file isn't a loadable executable (see `elfparse::ElfError` for exactly why).
    Elf(ElfError),
    /// The argument list doesn't fit in `ARG_MAX`.
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
/// (Stage 19's second resident slot) can reuse the first half unchanged.
pub struct PreparedProgram {
    /// The entry point of the program.
    entry: usize,
    /// The initial stack pointer of the program.
    sp: usize,
    /// The argument count.
    argc: usize,
    /// The argument vector (array of pointers to NUL-terminated strings).
    argv: usize,
}

/// Writes `items` onto the stack below `sp` as NUL-terminated C strings plus a `NULL`-terminated
/// array of pointers to them (see `argplan.rs` for the layout), and returns the array's address --
/// which is also the new stack pointer. `None` if `plan(&lens)` determines the items wouldn't
/// fit above `floor`.
///
/// SAFETY: `sp`/`floor` must lie inside the user window that `elf::load` just mapped, and nothing
/// may be running in it.
unsafe fn push_cstr_array(sp: usize, floor: usize, items: &[&str]) -> Option<usize> {
    // Collect the lengths of all items to plan the stack layout.
    let lens: Vec<usize> = items.iter().map(|s| s.len()).collect();

    // Plan the stack layout for the argument strings and the array of pointers.
    let plan = argplan::plan(sp, floor, &lens)?;

    // Copy each argument string into its planned location on the stack.
    for (item, &addr) in items.iter().zip(&plan.strings) {
        // SAFETY: `plan` keeps every string and the array within `floor..sp`.
        unsafe {
            core::ptr::copy_nonoverlapping(item.as_ptr(), addr as *mut u8, item.len());
            *((addr + item.len()) as *mut u8) = 0;
        }
    }

    // Write the array of pointers to the argument strings onto the stack.
    let array = plan.array as *mut usize;
    for (i, &addr) in plan.strings.iter().enumerate() {
        // SAFETY: the array's `len + 1` slots were carved out of the window by `plan`.
        unsafe { *array.add(i) = addr };
    }

    // Write the terminating `NULL` pointer for the array of argument strings.
    // SAFETY: as above -- the terminating `NULL` slot.
    unsafe { *array.add(plan.strings.len()) = 0 };

    // Return the address of the array of argument string pointers (the new stack pointer).
    Some(plan.array)
}

/// Loads `elf_bytes` and sets a fresh launch up, including the file descriptor table, exit status,
/// and initial stack with `args` as its `argv`. Returns a `PreparedProgram` on success.
pub fn prepare(elf_bytes: &[u8], args: &[&str]) -> Result<PreparedProgram, LaunchError> {
    // Set up the initial stack boundaries.
    let stack_top = USER_STACK_TOP;
    let floor = stack_top - ARG_MAX;

    // Dry run first: an argument list that can't fit must not cost a load.
    let lens: Vec<usize> = args.iter().map(|s| s.len()).collect();
    argplan::plan(stack_top, floor, &lens).ok_or(LaunchError::ArgsTooBig)?;

    // Load the ELF binary into memory and prepare the file descriptor table for the new program.
    let entry = elf::load(elf_bytes)?;

    // Reset the file descriptor table for the new program.
    fd::reset_for_launch();

    // Prepare the initial stack with the argument strings and array of pointers.
    //
    // SAFETY: `stack_top` and `floor` are inside the user window `elf::load` just mapped; the
    // destination is otherwise-unused stack memory nothing touches until the program itself runs.
    let argv = {
        let _user = crate::arch::mmu::user_access(); // writes the user stack, which PAN would forbid
        unsafe { push_cstr_array(stack_top, floor, args) }.ok_or(LaunchError::ArgsTooBig)?
    };
    Ok(PreparedProgram {
        entry,
        sp: argv,
        argc: args.len(),
        argv,
    })
}

/// Runs a `PreparedProgram` (from `prepare`) at EL0 to completion and returns its exit status -- `enter_el0`/
/// `resume_kernel` (`arch/context.s`) are what make an ordinary Rust function call correctly "pause"
/// for however long the EL0 program runs, however it ends.
///
/// The program runs with every DAIF bit masked -- no interrupt of any kind reaches it while
/// it's executing at EL0. This isn't a performance choice: a keyboard IRQ landing mid-program
/// would re-enter `handle_keyboard_irq` while this very call is still on the stack, and that
/// function's own line-editing path (`LINE`/`INPUT_ROW`/`launch`) isn't reentrant -- a second
/// `run_program` call from the nested invocation would remap the *same* fixed user window this
/// one is currently executing out of, and overwrite the single-slot `KERNEL_CTX` checkpoint
/// (`arch/context.s`) this call's own `enter_el0` just wrote. Masking DAIF here closes that off
/// entirely. A program that reads fd 0 isn't left deaf to the keyboard by this: `read(0)` drains
/// the device itself from inside the syscall (see `stdin.rs`), which is why that path doesn't
/// need the IRQ this masks. What's lost is only keystrokes typed while a program *isn't*
/// reading -- they queue up in the device -- and there is no Ctrl+C either way. (`Stage12.md`'s
/// Step 5 removes this structure: the eval loop leaves IRQ context, so the mask isn't needed.)
pub fn run(program: PreparedProgram) -> i32 {
    // M[3:0] = bits 3:0 = 0b0000 (EL0t). DAIF bits 9:6 are all *set* here (masked), not
    // cleared -- see this function's doc comment on why interrupts stay masked for a program's
    // entire time at EL0.
    const DAIF_MASKED: u64 = 0b1111 << 6; // D=A=I=F=1
    SPSR_EL1.set(DAIF_MASKED);

    ELR_EL1.set(program.entry as u64); // Set the entry point for the EL0 program
    SP_EL0.set(program.sp as u64); // The write-cursor's final position -- see `prepare`.

    // SAFETY: entry/SP_EL0 above point into the user window the loader just mapped, for
    // whichever program is being run this call; x0/x1 point at the argc/argv just written
    // there. Inline asm (rather than a plain `enter_el0()` call) specifically so x0/x1 are set
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
            enter = sym enter_el0,
            clobber_abi("C"),
        );
    }

    // Close every file the program left open, which finishes any writes still in progress.
    // Done while interrupts are still masked, like every other filesystem access made during a
    // launch.
    fd::end_launch();

    // Ensure that interrupts are unmasked for the kernel after the program has finished,
    // even if the program crashed or exited abnormally.
    DAIF.write(DAIF::I::CLEAR);

    status as i32
}

/// Loads and runs `elf_bytes` with `args` as its `argv`, returning its exit status -- `prepare`
/// then `run`. `Err` if it couldn't be started at all (nothing ran).
pub fn run_program(elf_bytes: &[u8], args: &[&str]) -> Result<i32, LaunchError> {
    Ok(run(prepare(elf_bytes, args)?))
}
