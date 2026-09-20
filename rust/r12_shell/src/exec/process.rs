//! Runs one EL0 program to completion (a normal `exit`, or a caught
//! fault) and returns to the caller like an ordinary function call. There
//! is no process table or scheduler here -- at most one program is ever
//! resident (see `arch/mmu.rs`'s own reasoning for why per-process machinery
//! has nothing to do in this system) -- but `run_program` still genuinely
//! *returns*, via the hand-rolled setjmp/longjmp in `arch/context.s`, rather
//! than requiring its caller to pass in "what happens next" as a
//! continuation.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering};

use aarch64_cpu::registers::{DAIF, ELR_EL1, SP_EL0, SPSR_EL1, Writeable};

use super::elfparse::ElfError;
use super::{argstack, elf};
use crate::platform::base_addresses::{USER_BASE, USER_SIZE};
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
    /// directly. Called from `sync_el0_handler` (`syscall.rs`) for `exit`
    /// and for a caught segfault alike; never returns itself.
    ///
    /// # Safety
    /// Must only be called from within the `sync_el0_64` handler, after
    /// `run_program` has actually started a program (so `arch/context.s`'s
    /// `KERNEL_CTX` holds a real checkpoint, not its zeroed initial
    /// state).
    pub fn resume_kernel() -> !;
}

/// The exit status of the program most recently run: what it passed to `exit`, or `EXIT_FAULT` if
/// it was killed by a fault instead. `run_program` reads it back once the program is over.
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// The status reported for a program stopped by a fault (`128 + SIGSEGV`, the shell convention),
/// since it never got to pass one to `exit` itself.
pub const EXIT_FAULT: i32 = 139;

/// Records the exit status for `run_program` to return -- called by the `exit` syscall and the
/// fault path in `syscall.rs`, just before they `resume_kernel`.
pub fn set_exit_code(code: i32) {
    EXIT_CODE.store(code, Ordering::Relaxed);
}

/// The most stack a program's initial `argv` (strings plus pointer array) may take -- far more than
/// a typed line can ever produce, and a small fraction of the 2 MiB user window, so an absurd
/// argument list is refused up front instead of running into the program's own memory.
pub const ARG_MAX: usize = 128 * 1024;

/// Why a program couldn't be started.
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
pub struct Prepared {
    entry: usize,
    sp: usize,
    argc: usize,
    argv: usize,
}

/// Writes `items` onto the stack below `sp` as NUL-terminated C strings plus a `NULL`-terminated
/// array of pointers to them (see `argstack.rs` for the layout), and returns the array's address --
/// which is also the new stack pointer. `None` if it wouldn't fit above `floor`.
///
/// The kernel is doing address bookkeeping on the *target* memory here: each pointer is written as
/// that string's user-stack address, not the kernel-side address the string was copied from -- an
/// easy, silent mistake that would only surface when the program dereferences `argv[1]`. `SP_EL0`
/// itself isn't touched: the write happens first, at addresses *above* where it will end up.
///
/// SAFETY: `sp`/`floor` must lie inside the user window that `elf::load` just mapped, and nothing
/// may be running in it.
unsafe fn push_cstr_array(sp: usize, floor: usize, items: &[&str]) -> Option<usize> {
    let lens: Vec<usize> = items.iter().map(|s| s.len()).collect();
    let plan = argstack::plan(sp, floor, &lens)?;
    for (item, &addr) in items.iter().zip(&plan.strings) {
        // SAFETY: `plan` keeps every string and the array within `floor..sp`.
        unsafe {
            core::ptr::copy_nonoverlapping(item.as_ptr(), addr as *mut u8, item.len());
            *((addr + item.len()) as *mut u8) = 0;
        }
    }
    let array = plan.array as *mut usize;
    for (i, &addr) in plan.strings.iter().enumerate() {
        // SAFETY: the array's `len + 1` slots were carved out of the window by `plan`.
        unsafe { *array.add(i) = addr };
    }
    // SAFETY: as above -- the terminating `NULL` slot.
    unsafe { *array.add(plan.strings.len()) = 0 };
    Some(plan.array)
}

/// Loads `elf_bytes` and sets a fresh launch up: the fd table (`fd::reset_for_launch`), the exit
/// status, and the initial stack holding `args` as its `argv` (`args[0]` is the program's own name,
/// same C convention; `argv[argc]` is `NULL`). Nothing is touched if the program can't be started:
/// the argument list is checked against `ARG_MAX` first, and `elf::load` validates the whole file
/// before copying or mapping anything.
///
/// The argc/argv stack setup (see `ROADMAP.md`'s Stage 10 section): starting from `stack_top` and
/// working *downward*, `push_cstr_array` writes each argument string and then the pointer array,
/// 16-byte-aligned since it becomes the program's own incoming `SP_EL0` -- a real AAPCS64 call
/// boundary, `main(argc, argv)` receiving it via `x0`/`x1`. With `args` empty this degrades to just
/// the `NULL` slot.
pub fn prepare(elf_bytes: &[u8], args: &[&str]) -> Result<Prepared, LaunchError> {
    let stack_top = USER_BASE + USER_SIZE;
    let floor = stack_top - ARG_MAX;

    // Dry run first: an argument list that can't fit must not cost a load.
    let lens: Vec<usize> = args.iter().map(|s| s.len()).collect();
    argstack::plan(stack_top, floor, &lens).ok_or(LaunchError::ArgsTooBig)?;

    let entry = elf::load(elf_bytes)?;
    fd::reset_for_launch();
    set_exit_code(0);

    // SAFETY: `stack_top` and `floor` are inside the user window `elf::load` just mapped; the
    // destination is otherwise-unused stack memory nothing touches until the program itself runs.
    let argv = unsafe { push_cstr_array(stack_top, floor, args) }.ok_or(LaunchError::ArgsTooBig)?;
    Ok(Prepared { entry, sp: argv, argc: args.len(), argv })
}

/// Runs a `prepare`d program at EL0 to completion and returns its exit status -- `enter_el0`/
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
pub fn run(program: Prepared) -> i32 {
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
    unsafe {
        core::arch::asm!(
            "bl {enter}",
            in("x0") program.argc,
            in("x1") program.argv,
            enter = sym enter_el0,
            clobber_abi("C"),
        );
    }

    // Commit whatever files the program left open while interrupts are still masked, like every
    // other filesystem access made during a launch -- unmasked below, a stale block-device IRQ
    // (pending since the program's own masked reads, never acknowledged) would be taken in the
    // middle of these writes.
    fd::end_launch();

    // The program has exited (SYS_EXIT) or faulted -- either way, resume_kernel (arch/context.s)
    // jumped back into enter_el0's own return point via a raw branch, not an eret, so it never
    // restored DAIF the way returning from an exception normally would. This is a separate
    // concern from SPSR_EL1's masking above (that's about EL0's own execution; this is about
    // *this kernel's* state once back in an ordinary EL1 call chain, which needs DAIF unmasked
    // to keep handling keyboard IRQs at all): taking the syscall/fault exception that got us
    // here already masked it architecturally (entering EL1 always does, regardless of what
    // SPSR_EL1 said for EL0), and left alone, the shell would go deaf to every further keystroke
    // after the very first program ever runs.
    // Re-clear it explicitly, same as kernel_main's own one-time "DAIF stays unmasked from here
    // on".
    DAIF.write(DAIF::I::CLEAR);

    EXIT_CODE.load(Ordering::Relaxed)
}

/// Loads and runs `elf_bytes` with `args` as its `argv`, returning its exit status -- `prepare`
/// then `run`. `Err` if it couldn't be started at all (nothing ran).
pub fn run_program(elf_bytes: &[u8], args: &[&str]) -> Result<i32, LaunchError> {
    Ok(run(prepare(elf_bytes, args)?))
}
