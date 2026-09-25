//! Loading and starting programs: validating and mapping an ELF (`elfparse`, `elf`), laying out
//! `argv` and `envp` on the new stack (`argplan`), and entering and leaving EL0 (`process`); and the shell state a
//! program starts with -- working directory and standard streams (`frame_stack`, `shell_state`).

pub mod argplan;
pub mod elf;
pub mod elfparse;
pub mod frame_stack;
pub mod process;
pub mod shell_state;
pub mod usermem;
