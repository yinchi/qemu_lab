//! `spin N` -- busy-waits N seconds (default 1) without reading anything, then prints `spun N`. There is no
//! sleep syscall, so this is how a test keeps a program running "for a while" -- for typing during it,
//! for instance (`Stage12.md`'s Step 5). It reads the virtual counter, which the kernel makes readable at
//! EL0.

#![no_std]
#![no_main]

use core::arch::asm;
use core::fmt::Write;

use progs::Fd;

userlib::entry_with_args!(run);

fn counter() -> u64 {
    let value: u64;
    // SAFETY: reads the virtual counter, enabled for EL0 by the kernel (`CNTKCTL_EL1.EL0VCTEN`).
    unsafe { asm!("mrs {v}, cntvct_el0", v = out(reg) value) };
    value
}

fn run(mut args: userlib::Args) {
    let _ = args.next(); // argv[0]
    let seconds = args.next().and_then(progs::atoi).unwrap_or(1) as u64;
    let frequency: u64;
    // SAFETY: reads the counter frequency, always readable at EL0.
    unsafe { asm!("mrs {f}, cntfrq_el0", f = out(reg) frequency) };
    let start = counter();
    while counter() - start < frequency * seconds {
        core::hint::spin_loop();
    }
    let _ = writeln!(Fd(1), "spun {seconds}");
}
