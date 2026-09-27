//! The pure logic behind `edit`, Stage 20's editor: the line buffer, the soft-wrap layout, the
//! view's scrolling, the mark and cut buffer, and the `.editrc` parser. `no_std` + `alloc`, no
//! dependency on `userlib` or any syscall, so every module here is host-tested
//! (`r20_editor/hosttests`) exactly like the kernel's own pure modules.

#![no_std]

extern crate alloc;

pub mod buffer;
pub mod editrc;
pub mod layout;
pub mod region;
pub mod render;
pub mod scroll;
pub mod width;
