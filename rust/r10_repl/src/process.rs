//! Runs one EL0 program to completion (a normal `exit`, or a caught
//! fault) and returns to the caller like an ordinary function call. There
//! is no process table or scheduler here -- at most one program is ever
//! resident (see `mmu.rs`'s own reasoning for why per-process machinery
//! has nothing to do in this system) -- but `run_program` still genuinely
//! *returns*, via the hand-rolled setjmp/longjmp in `process.s`, rather
//! than requiring its caller to pass in "what happens next" as a
//! continuation.

use alloc::vec::Vec;

use aarch64_cpu::registers::{DAIF, ELR_EL1, SP_EL0, SPSR_EL1, Writeable};

use crate::base_addresses::{USER_BASE, USER_SIZE};

unsafe extern "C" {
    /// Checkpoints this project's callee-saved register set and `eret`s
    /// into EL0 (`SPSR_EL1`/`ELR_EL1`/`SP_EL0` already set by
    /// `run_program`, below). Upholds the ordinary AAPCS64 calling
    /// convention exactly, so calling it from Rust needs no special
    /// handling -- see `process.s`'s own doc comment for the full
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
    /// `run_program` has actually started a program (so `process.s`'s
    /// `KERNEL_CTX` holds a real checkpoint, not its zeroed initial
    /// state).
    pub fn resume_kernel() -> !;
}

/// Loads `elf_bytes` and runs it at EL0 to completion with `args` as its `argv` (`args[0]` is
/// the program's own name, same C convention), then returns normally -- `enter_el0`/
/// `resume_kernel` (`process.s`) are what make an ordinary Rust function call correctly "pause"
/// for however long the EL0 program runs, however it ends.
///
/// The argc/argv stack setup (see `ROADMAP.md`'s Stage 10 section): starting from `stack_top`
/// and working *downward*, each argument string is written (NUL-terminated, C-string style --
/// `userlib::Args` scans for that terminator), then the pointer array referencing them, 16-byte-
/// aligned since it becomes the program's own incoming `SP_EL0` -- a real AAPCS64 call boundary,
/// `main(argc, argv)` receiving it via `x0`/`x1`. Wherever that array's base address ends up is
/// the new `SP_EL0`; with `args` empty this degrades to no writes at all and `SP_EL0` staying at
/// `stack_top`, the same value Stage 9's original (argv-less) `run_program` always used.
///
/// The program runs with every DAIF bit masked -- no interrupt of any kind reaches it while
/// it's executing at EL0. This isn't a performance choice: a keyboard IRQ landing mid-program
/// would re-enter `handle_keyboard_irq` while this very call is still on the stack, and that
/// function's own line-editing path (`LINE`/`INPUT_ROW`/`launch`) isn't reentrant -- a second
/// `run_program` call from the nested invocation would remap the *same* fixed user window this
/// one is currently executing out of, and overwrite the single-slot `KERNEL_CTX` checkpoint
/// (`process.s`) this call's own `enter_el0` just wrote. Masking DAIF here closes that off
/// entirely, at the cost of no live keystroke feedback while a program runs -- an acceptable
/// cost for a single-tasking kernel with no scheduler and no Ctrl+C yet either way. Unmasked
/// again only for a program that's earned it: Stage 13's raw-mode toggle (which redirects
/// `handle_keyboard_irq`'s tokens away from the canonical path entirely, so the reentrancy this
/// masking exists to prevent doesn't apply to it in the first place), or real preemptive
/// scheduling (Stage 24), whichever gets built first.
pub fn run_program(elf_bytes: &[u8], args: &[&str]) {
    let entry = crate::elf::load(elf_bytes);

    let stack_top = USER_BASE + USER_SIZE;
    let mut sp = stack_top;

    // Write each argument string, NUL-terminated, working downward from stack_top. This is the
    // kernel doing address bookkeeping on the *target* memory -- SP_EL0 itself isn't touched
    // until the very end, below.
    let mut str_addrs = Vec::with_capacity(args.len());
    for arg in args {
        sp -= arg.len() + 1; // +1 for the NUL terminator every argv entry needs.
        // SAFETY: sp stays within the user window `elf::load` just mapped -- LINE's own
        // screen-width limit (see main.rs's show_row) already bounds how long a typed line, and
        // so every argument sliced from it, can possibly be, far short of the 2 MiB window. The
        // destination is otherwise-unused stack memory nothing touches until the program itself
        // runs.
        unsafe {
            core::ptr::copy_nonoverlapping(arg.as_ptr(), sp as *mut u8, arg.len());
            *((sp + arg.len()) as *mut u8) = 0;
        }
        str_addrs.push(sp);
    }

    // The pointer array itself, argc entries of usize, 16-byte-aligned -- see this function's
    // doc comment for why.
    sp = (sp - args.len() * core::mem::size_of::<usize>()) & !0xf;
    let argv = sp as *mut usize;
    for (i, &addr) in str_addrs.iter().enumerate() {
        // SAFETY: argv..argv+argc*8 was just carved out of the user window above, exclusively
        // for this array.
        unsafe { *argv.add(i) = addr };
    }

    // M[3:0] = bits 3:0 = 0b0000 (EL0t). DAIF bits 9:6 are all *set* here (masked), not
    // cleared -- see this function's doc comment on why interrupts stay masked for a program's
    // entire time at EL0.
    const DAIF_MASKED: u64 = 0b1111 << 6; // D=A=I=F=1
    SPSR_EL1.set(DAIF_MASKED);

    ELR_EL1.set(entry as u64); // Set the entry point for the EL0 program
    SP_EL0.set(sp as u64); // The write-cursor's final position -- see this function's doc comment.

    // SAFETY: entry/SP_EL0 above point into the user window the loader just mapped, for
    // whichever program is being run this call; x0/x1 point at the argc/argv just written
    // there. Inline asm (rather than a plain `enter_el0()` call) specifically so x0/x1 are set
    // in the same instruction sequence that calls it, with nothing of Rust's own codegen
    // between the two free to reuse those (ordinarily caller-saved) registers first.
    unsafe {
        core::arch::asm!(
            "bl {enter}",
            in("x0") args.len(),
            in("x1") argv,
            enter = sym enter_el0,
            clobber_abi("C"),
        );
    }

    // The program has exited (SYS_EXIT) or faulted -- either way, resume_kernel (process.s)
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
}
