//! What the Stage 19 programs share: `table`, the column layout `lsblk` prints (pure, so it is host-tested like the
//! kernel's pure modules).

#![no_std]

extern crate alloc;

pub mod table;
