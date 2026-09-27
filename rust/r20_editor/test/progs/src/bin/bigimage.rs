//! `bigimage` -- a program whose image is far bigger than the 2 MiB window programs had before Stage 15:
//! a 2 MiB initialized `.data` table, 1 MiB of read-only data and 8 MiB of `.bss`, about 11 MiB of memory in
//! all (the file is about 3 MiB; `.bss` is not in it). It checks every byte range it owns and prints one
//! line for each, so a test can tell the whole image really was mapped, filled and zeroed correctly.
//!
//!   data: SUM     the sum of the `.data` table (every word starts as 0xA5A5A5A5), read back after
//!                 the kernel copied it in from the file
//!   rodata: SUM   the sum of the read-only table (every byte 7)
//!   bss: N pages  every page of the 8 MiB `.bss` was zero, was written, and read back what was written

#![no_std]
#![no_main]

use core::fmt::Write;
use core::hint::black_box;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

use progs::Fd;

userlib::entry!(run);

const DATA_WORDS: usize = 512 * 1024; // 2 MiB
const RODATA_BYTES: usize = 1024 * 1024;
const BSS_BYTES: usize = 8 * 1024 * 1024;
const PAGE: usize = 4096;

static mut DATA: [u32; DATA_WORDS] = [0xA5A5_A5A5; DATA_WORDS];
static RODATA: [u8; RODATA_BYTES] = [7; RODATA_BYTES];
static mut BSS: [u8; BSS_BYTES] = [0; BSS_BYTES];

fn run() {
    let mut out = Fd(1);

    let mut sum = 0u64;
    let data = black_box(addr_of!(DATA)) as *const u32;
    for i in 0..DATA_WORDS {
        // SAFETY: in bounds of `DATA`.
        sum += u64::from(unsafe { read_volatile(data.add(i)) });
    }
    let _ = writeln!(out, "data: {sum}");

    let mut sum = 0u64;
    let rodata = black_box(addr_of!(RODATA)) as *const u8;
    for i in 0..RODATA_BYTES {
        // SAFETY: in bounds of `RODATA`.
        sum += u64::from(unsafe { read_volatile(rodata.add(i)) });
    }
    let _ = writeln!(out, "rodata: {sum}");

    let bss = black_box(addr_of_mut!(BSS)) as *mut u8;
    let mut pages = 0;
    for page in 0..BSS_BYTES / PAGE {
        let first = page * PAGE;
        let last = first + PAGE - 1;
        // SAFETY: in bounds of `BSS`.
        unsafe {
            if read_volatile(bss.add(first)) != 0 || read_volatile(bss.add(last)) != 0 {
                let _ = writeln!(out, "bss: page {page} was not zero");
                return;
            }
            write_volatile(bss.add(first), (page % 251) as u8 + 1);
            write_volatile(bss.add(last), (page % 241) as u8 + 1);
            if read_volatile(bss.add(first)) != (page % 251) as u8 + 1
                || read_volatile(bss.add(last)) != (page % 241) as u8 + 1
            {
                let _ = writeln!(out, "bss: page {page} read back wrong");
                return;
            }
        }
        pages += 1;
    }
    let _ = writeln!(out, "bss: {pages} pages");
}
