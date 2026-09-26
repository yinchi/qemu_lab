//! What the Stage 19 programs share, pure so it is host-tested like the kernel's pure modules: `table`, the column layout
//! `lsblk` prints, and `spell`, how `mount` names a volume.

#![no_std]

extern crate alloc;

pub mod spell;
pub mod table;
