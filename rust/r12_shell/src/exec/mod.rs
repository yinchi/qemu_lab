//! Loading and starting programs: validating and mapping an ELF (`elfparse`, `elf`), laying out
//! `argv` on the new stack (`argplan`), and entering and leaving EL0 (`process`).

pub mod argplan;
pub mod elf;
pub mod elfparse;
pub mod process;
pub mod usermem;
