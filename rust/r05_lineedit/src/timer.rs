//! EL1 physical timer driver for the ARMv8-A architecture.
//! Provides functions to configure and control the EL1 physical timer.

// Note: registers have suffix `_EL0` to indicate they are accessible at EL0,
// even though this driver operates at EL1.  Setting CNTKCTL_EL1.EL0PTEN
// (CouNter-Timer Kernel ConTroL, bit 9) would enable EL0 access to the timer -- not needed yet.

use aarch64_cpu::registers::{CNTFRQ_EL0, CNTP_CTL_EL0, CNTP_TVAL_EL0, Readable, Writeable};

/// PPI number for the non-secure EL1 physical timer (`CNTP_*` registers)
/// interrupt -- converted into a full interrupt ID via `IntId::ppi(n)`.
/// Confirmed via device-tree dump : PPI 14 -> interrupt ID 16+14 = 30.
pub const PPI: u32 = 14;

/// Reads `CNTFRQ_EL0`: the system counter frequency in Hz, fixed by the platform at boot.
pub fn freq() -> u64 {
    CNTFRQ_EL0.get()
}

/// Arms (or re-arms) the EL1 physical timer to fire `ticks` counter cycles from now.
pub fn arm(ticks: u64) {
    CNTP_TVAL_EL0.set(ticks);
    CNTP_CTL_EL0.write(CNTP_CTL_EL0::ENABLE::SET);
}

/// Disables the physical timer, deasserting its interrupt line.
pub fn disable() {
    CNTP_CTL_EL0.write(CNTP_CTL_EL0::ENABLE::CLEAR);
}
