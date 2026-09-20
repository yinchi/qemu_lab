//! Loading and starting programs: validating and mapping an ELF (`elfparse`, `elf`), laying out
//! `argv` on the new stack (`argstack`), and entering and leaving EL0 (`process`).

pub mod argstack;
pub mod elf;
pub mod elfparse;
pub mod process;
