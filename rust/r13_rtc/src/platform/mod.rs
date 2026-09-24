//! The QEMU `virt` board: where its devices are (parsed from the device tree), the UART, the real-time
//! clock, and the kernel's global device statics.

pub mod base_addresses;
pub mod globals;
pub mod rtc;
pub mod uart;
