//! `SVC`-based syscalls and the segfault path, both reached from
//! `sync_el0_64` (`arch/vectors.s`) via `sync_el0_handler`. The fd table those syscalls act on is
//! in `syscall/fd.rs`.

pub mod fd;

use aarch64_cpu::registers::{ESR_EL1, FAR_EL1, Readable};

use crate::exec::process;
use crate::platform::uart::{uart_ensure_newline, uart_write};

// Syscall numbers come from the shared `abi` crate (Linux's real aarch64 values, borrowed for
// familiarity -- see its module doc), the same ones `userlib` issues them with.
use abi::errno::ENOSYS;
use abi::syscall::{SYS_CHMOD, SYS_CLOSE, SYS_EXIT, SYS_GETDENTS, SYS_OPEN, SYS_READ, SYS_WRITE};

// The ESR_EL1 EC field values this handler decodes -- see sync_el0_handler for what each one
// means here.
const EC_SVC64: u64 = 0x15;
const EC_IABT_LOWER: u64 = 0x20;
const EC_DABT_LOWER: u64 = 0x24;

/// Mirrors `kernel_entry`'s stack layout exactly (`arch/vectors.s`): all 31 GPRs
/// (`x0`-`x30`), then the `ELR_EL1`/`SPSR_EL1` pair saved on entry.
/// `sync_el0_64` passes the frame's base address in `x0` before calling
/// `sync_el0_handler`, so this struct is how Rust reads the syscall's
/// number/arguments back out of it, and writes the return value back in
/// before `kernel_exit` restores everything.
#[repr(C)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub elr_el1: u64,
    pub spsr_el1: u64,
}

/// Called from `sync_el0_64` with the trap frame's address in `x0`.
/// Decodes `ESR_EL1`'s `EC` field to tell a deliberate syscall apart from
/// a fault -- see the three arms below for what each one does.
#[unsafe(no_mangle)]
extern "C" fn sync_el0_handler(regs: *mut TrapFrame) {
    let esr = ESR_EL1.get(); // read the Exception Syndrome Register (ESR_EL1)
    let ec = (esr >> 26) & 0x3f; // extract the Exception Class (EC) field from ESR_EL1

    match ec {
        // EC_SVC64 indicates a 64-bit SVC (syscall) from EL0.
        EC_SVC64 => {
            // SAFETY: regs points at kernel_entry's just-saved frame,
            // still live on the exception stack -- sole access to it here.
            let regs = unsafe { &mut *regs };
            let nr = regs.x[8] as usize; // syscall number
            let a0 = regs.x[0] as usize; // first argument
            let a1 = regs.x[1] as usize; // second argument
            let a2 = regs.x[2] as usize; // third argument
            let a3 = regs.x[3] as usize; // fourth argument

            // Dispatch the syscall based on its number.
            match nr {
                SYS_WRITE => regs.x[0] = fd::write(a0, a1, a2) as u64,
                SYS_READ => regs.x[0] = fd::read(a0, a1, a2) as u64,
                SYS_OPEN => regs.x[0] = fd::open(a0, a1, a2) as u64,
                SYS_CLOSE => regs.x[0] = fd::close(a0) as u64,
                SYS_GETDENTS => regs.x[0] = fd::getdents(a0, a1, a2) as u64,
                SYS_CHMOD => regs.x[0] = fd::chmod(a0, a1, a2, a3) as u64,
                SYS_EXIT => {
                    process::set_exit_code(a0 as i32);
                    // Never returns to kernel_exit's normal eret-back-to-EL0
                    // path -- resume_kernel (arch/context.s) restores the register
                    // set enter_el0 checkpointed and jumps straight back into
                    // run_program's call site instead.
                    // SAFETY: only reachable once run_program has actually
                    // called enter_el0 (context.s's KERNEL_CTX holds a real
                    // checkpoint, not its zeroed initial state).
                    unsafe { process::resume_kernel() }
                }
                _ => regs.x[0] = ENOSYS as u64, // no such syscall
            }
        }

        // EC_IABT_LOWER and EC_DABT_LOWER indicate instruction and data aborts from EL0,
        // respectively.
        EC_IABT_LOWER | EC_DABT_LOWER => {
            let far = FAR_EL1.get(); // read the Fault Address Register (FAR_EL1)
            // UART only, not the GPU console: unlike ordinary program output (fd::write, which
            // goes through Console and keeps INPUT_ROW in sync -- see syscall/fd.rs), a segfault is an
            // unplanned interruption mid-program: writing through Console directly here too
            // would move its cursor without run_program's caller ever getting a chance to
            // resync INPUT_ROW to it. UART-only matches this crate's own panic handler's choice
            // for the same reason.
            uart_ensure_newline();
            uart_write(
                alloc::format!("Segmentation fault (address {far:#x}, ESR_EL1 {esr:#x})\n")
                    .as_bytes(),
            );
            process::set_exit_code(process::EXIT_FAULT);
            // SAFETY: only reachable once run_program has actually called
            // enter_el0 (context.s's KERNEL_CTX holds a real checkpoint, not
            // its zeroed initial state).
            unsafe { process::resume_kernel() }
        }
        _ => {
            // Anything else (FP exceptions, alignment faults, etc.) --
            // deliberately narrow decoding, not a general fault-handling
            // framework. `8` is sync_el0_64's own index into
            // unexpected_exception's ERROR_TYPES table.
            crate::unexpected_exception(8)
        }
    }
}
