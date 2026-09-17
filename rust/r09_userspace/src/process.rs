//! Runs one EL0 program to completion (a normal `exit`, or a caught
//! fault) and returns to the caller like an ordinary function call. There
//! is no process table or scheduler here -- at most one program is ever
//! resident (see `mmu.rs`'s own reasoning for why per-process machinery
//! has nothing to do in this system) -- but `run_program` still genuinely
//! *returns*, via the hand-rolled setjmp/longjmp in `process.s`, rather
//! than requiring its caller to pass in "what happens next" as a
//! continuation.

use aarch64_cpu::registers::{ELR_EL1, SP_EL0, SPSR_EL1, Writeable};

use crate::mmu::{USER_BASE, USER_SIZE};

unsafe extern "C" {
    /// Checkpoints this project's callee-saved register set and `eret`s
    /// into EL0 (`SPSR_EL1`/`ELR_EL1`/`SP_EL0` already set by
    /// `run_program`, below). Upholds the ordinary AAPCS64 calling
    /// convention exactly, so calling it from Rust needs no special
    /// handling -- see `process.s`'s own doc comment for the full
    /// reasoning.
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

/// Loads `elf_bytes` and runs it at EL0 to completion, then returns
/// normally -- `enter_el0`/`resume_kernel` (`process.s`) are what make an
/// ordinary Rust function call correctly "pause" for however long the EL0
/// program runs, however it ends.
pub fn run_program(elf_bytes: &[u8]) {
    let entry = crate::elf::load(elf_bytes);

    // M[3:0] = bits 3:0 = 0b0000 (EL0t), every DAIF bit unmasked (bits 9:6 = 0)
    SPSR_EL1.set(0); 

    ELR_EL1.set(entry as u64); // Set the entry point for the EL0 program
    SP_EL0.set((USER_BASE + USER_SIZE) as u64); // Set the stack pointer for the EL0 program

    // SAFETY: entry/SP_EL0 above point into the user window the loader
    // just mapped, for whichever program is being run this call.
    unsafe {
        enter_el0();
    }
}
