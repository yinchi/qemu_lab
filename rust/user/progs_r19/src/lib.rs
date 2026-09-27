//! What the Stage 19 programs share: `copy`, the engine behind `cp` (and `mv` between volumes), over the file syscalls; and,
//! pure so they are host-tested like the kernel's pure modules, `table` (the column layout `lsblk` prints), `spell` (how
//! `mount` names a volume) and `stamp` (the times `touch` reads and FAT stores).

#![no_std]

extern crate alloc;

pub mod copy;
pub mod spell;
pub mod stamp;
pub mod table;
