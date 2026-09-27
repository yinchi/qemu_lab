//! Powers off or restarts the machine via PSCI (Power State Coordination Interface) -- the firmware
//! service QEMU's `virt` machine implements above EL1, reached with `hvc` (hypervisor call), the
//! conduit `virt` uses by default. Both functions below are documented never to return; the
//! trailing `hang()` is defensive only, the same fallback the panic handler uses, in case something
//! intercepts the call without actually powering off or resetting the board.

const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

/// Powers off the machine.
pub fn system_off() -> ! {
    psci_call(PSCI_SYSTEM_OFF);
    crate::hang()
}

/// Restarts the machine.
pub fn system_reset() -> ! {
    psci_call(PSCI_SYSTEM_RESET);
    crate::hang()
}

/// Issues `hvc #0` with `function` in `x0`, per PSCI's SMCCC-based calling convention: up to four
/// arguments in `x0`-`x3`, all clobbered by the call. Neither caller above passes further
/// arguments, so `x1`-`x3` are left undefined on entry.
fn psci_call(function: u64) {
    // SAFETY: HVC #0 traps to the firmware/hypervisor layer QEMU's `virt` machine runs above EL1,
    // which implements PSCI. Not marked `noreturn`: SYSTEM_OFF/SYSTEM_RESET are documented never to
    // return, but that's a property of the call, not something the compiler can assume -- both
    // callers still fall through to `hang()` in case it does.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            in("x0") function,
            out("x1") _,
            out("x2") _,
            out("x3") _,
            options(nomem, nostack),
        );
    }
}
