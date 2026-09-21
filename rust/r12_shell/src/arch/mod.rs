//! AArch64 specifics: the interrupt controller, interrupt masking and the MMU. (The assembly -- `boot.s`,
//! `vectors.s`, `context.s` -- lives alongside in `arch/`; `build.rs` assembles it.)

pub mod gic;
pub mod irq;
pub mod mmu;
