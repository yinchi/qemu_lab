//! `heapuse` -- exercises the user heap (`userlib`'s `heap` feature: `alloc` over `brk`) the way a real program
//! would: one big `Vec`, thousands of small boxes freed and reused, a string built up piece by piece, a `Vec`
//! that grows by doubling, an allocation the ceiling cannot hold (refused cleanly), and memory reuse after a
//! free. Prints one line for each.
//!
//!   big: SUM        an 8 MiB `Vec` filled with a pattern, summed
//!   small: SUM      20000 boxed integers, every other one freed, then reallocated, summed
//!   string: LEN     10000 `format!`ed pieces pushed onto a `String`
//!   grow: LEN SUM   a `Vec<u32>` pushed to 500000 elements (about 20 reallocations), summed
//!   oom: refused    a 64 MiB reservation, which cannot fit under the 32 MiB window, failed without a panic
//!   reuse: yes      freeing the 8 MiB `Vec` and allocating another 8 MiB did not push the break up by another 8 MiB
//!   heap: N KiB     how far the break moved in all (proves the heap grew on demand, not up front)

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use progs::Fd;

userlib::entry!(run);

fn run() {
    let mut out = Fd(1);
    let first = userlib::brk(0);

    let mut big: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    for i in 0..8 * 1024 * 1024usize {
        big.push((i % 251) as u8);
    }
    let sum: u64 = big.iter().map(|&b| u64::from(b)).sum();
    let _ = writeln!(out, "big: {sum}");

    let mut boxes: Vec<Option<Box<u64>>> = (0..20_000u64).map(|i| Some(Box::new(i))).collect();
    for slot in boxes.iter_mut().step_by(2) {
        *slot = None; // freed
    }
    for (i, slot) in boxes.iter_mut().enumerate().step_by(2) {
        *slot = Some(Box::new(i as u64 * 2)); // reallocated into the holes
    }
    let sum: u64 = boxes.iter().flatten().map(|b| **b).sum();
    let _ = writeln!(out, "small: {sum}");
    drop(boxes);

    let mut text = String::new();
    for i in 0..10_000 {
        text.push_str(&format!("[{i}]"));
    }
    let _ = writeln!(out, "string: {}", text.len());
    drop(text);

    let mut numbers: Vec<u32> = Vec::new();
    for i in 0..500_000u32 {
        numbers.push(i);
    }
    let sum: u64 = numbers.iter().map(|&n| u64::from(n)).sum();
    let _ = writeln!(out, "grow: {} {sum}", numbers.len());
    drop(numbers);

    let mut too_big: Vec<u8> = Vec::new();
    let refused = too_big.try_reserve(64 * 1024 * 1024).is_err();
    let _ = writeln!(out, "oom: {}", if refused { "refused" } else { "GRANTED" });

    // Free the big vector and take the same again: the heap reuses it, and the break barely moves.
    drop(big);
    let before = userlib::brk(0);
    let mut again: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024);
    again.resize(8 * 1024 * 1024, 1);
    let _ = writeln!(out, "reuse: {}", if userlib::brk(0) <= before + 1024 * 1024 { "yes" } else { "no" });
    let _ = writeln!(out, "heap: grew {}", if userlib::brk(0) > first { "on demand" } else { "not at all" });
}
