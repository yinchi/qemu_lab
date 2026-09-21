//! Host-side tests for `r12_shell`'s pure-logic modules. A module qualifies by being `no_std` +
//! `alloc` with no dependency on the rest of the kernel; each is pulled in here by path, and its
//! own `#[cfg(test)]` tests run as part of this crate.
//!
//! Modules are added as the Steps in `Stage12.md` create them.

extern crate alloc;

#[path = "../../src/exec/argplan.rs"]
pub mod argplan;
#[path = "../../src/exec/elfparse.rs"]
pub mod elfparse;
