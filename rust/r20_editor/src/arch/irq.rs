//! Masking and waiting for interrupts, for code that shares state with an interrupt handler.

/// Runs `f` with IRQs masked, then puts the mask back the way it was -- so it is safe to call both with
/// IRQs on (the shell's loop) and with them already off (inside a syscall).
pub fn without_irqs<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    // SAFETY: reads DAIF, then masks IRQs (`daifset #2` sets the I bit).
    unsafe { core::arch::asm!("mrs {d}, daif", "msr daifset, #2", d = out(reg) daif) };
    let result = f();
    const I: u64 = 1 << 7;
    if daif & I == 0 {
        // SAFETY: IRQs were unmasked when we came in; `daifclr #2` clears the I bit again.
        unsafe { core::arch::asm!("msr daifclr, #2") };
    }
    result
}

/// Sleeps until an interrupt arrives, unless `ready()` already says there is something to do -- the
/// check and the sleep are done with IRQs masked, so an interrupt that lands in between is not lost:
/// it stays pending, `wfi` returns at once, and it is taken as soon as the mask is lifted. Returns
/// with IRQs unmasked.
pub fn wait_for_interrupt_unless(ready: impl FnOnce() -> bool) {
    // SAFETY: masks IRQs; `wfi` wakes on a pending interrupt even while it is masked.
    unsafe {
        core::arch::asm!("msr daifset, #2");
        if !ready() {
            core::arch::asm!("wfi");
        }
        core::arch::asm!("msr daifclr, #2");
    }
}
