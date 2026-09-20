//! `probe` -- a test-only program that pokes at the kernel's syscall surface from EL0, so the
//! harness can check what the kernel does with input a well-behaved program never sends. Each Step of
//! `Stage12.md` that needs a new probe adds a subcommand here; see `test/README.md`.
//!
//!   probe sys-unknown   an unassigned syscall number
//!   probe bad-ptr       bad or wrapping pointers/lengths given to write/read/open/chmod
//!   probe fds           opens files until the kernel says no, then closes them all
//!   probe args ...      prints argc/argv exactly as received, plus what the stack layout guarantees

#![no_std]
#![no_main]

use core::arch::asm;
use core::fmt::Write;

use abi::syscall::{SYS_CHMOD, SYS_GETDENTS, SYS_OPEN, SYS_READ, SYS_WRITE};
use progs::Fd;

/// A raw syscall with up to four arguments -- deliberately not `userlib`'s, which only ever sends
/// well-formed calls.
fn raw(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    // SAFETY: `svc` with the AArch64/Linux convention `userlib` documents; the kernel preserves
    // every register but `x0`.
    unsafe {
        asm!(
            "svc #0",
            in("x8") nr,
            inout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
        );
    }
    ret
}

#[unsafe(no_mangle)]
pub extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
    // SAFETY: `_start` forwards the kernel's argc/argv untouched (see userlib's `args`).
    let code = run(unsafe { userlib::args(argc, argv) }, argc, argv);
    userlib::exit(code)
}

fn run(mut args: userlib::Args, argc: usize, argv: *const *const u8) -> i32 {
    let _ = args.next(); // argv[0]
    let mut out = Fd(1);
    match args.next() {
        Some("sys-unknown") => {
            let _ = writeln!(out, "unknown syscall: {}", raw(9999, 0, 0, 0, 0));
            0
        }
        Some("bad-ptr") => {
            bad_ptr(&mut out);
            0
        }
        Some("fds") => {
            fds(&mut out);
            0
        }
        Some("args") => {
            print_args(&mut out, argc, argv);
            0
        }
        _ => {
            let _ = writeln!(Fd(2), "usage: probe sys-unknown|bad-ptr|fds|args ...");
            2
        }
    }
}

fn bad_ptr(out: &mut Fd) {
    let valid = b"hello".as_ptr() as usize;
    let cases: [(&str, isize); 8] = [
        ("write, pointer in kernel memory", raw(SYS_WRITE, 1, 0x4000_0000, 4, 0)),
        ("write, pointer at the top of the address space", raw(SYS_WRITE, 1, usize::MAX - 3, 8, 0)),
        ("write, length larger than the window", raw(SYS_WRITE, 1, valid, 0x4000_0000, 0)),
        ("read, bad pointer", raw(SYS_READ, 0, 0xffff_0000_0000_0000, 4, 0)),
        ("open, bad pointer", raw(SYS_OPEN, 0x1000, 5, 0, 0)),
        ("open, path is not UTF-8", raw(SYS_OPEN, b"\xff\xfe".as_ptr() as usize, 2, 0, 0)),
        ("chmod, bad pointer", raw(SYS_CHMOD, usize::MAX, 4, 0, 0)),
        ("getdents on a closed fd", raw(SYS_GETDENTS, 99, valid, 8, 0)),
    ];
    for (what, ret) in cases {
        let _ = writeln!(out, "{what}: {ret}");
    }
}

fn fds(out: &mut Fd) {
    let mut opened = 0usize;
    let mut fds = [0usize; 32];
    let stopped_by = loop {
        let fd = userlib::open("/tests/notes.txt", userlib::O_RDONLY);
        if fd < 0 || opened == fds.len() {
            break fd;
        }
        fds[opened] = fd as usize;
        opened += 1;
    };
    let _ = writeln!(out, "opened {opened}, then {stopped_by}");
    for &fd in &fds[..opened] {
        userlib::close(fd);
    }
    let again = userlib::open("/tests/notes.txt", userlib::O_RDONLY);
    let _ = writeln!(out, "after closing all: {}", if again >= 0 { "open ok" } else { "open failed" });
    userlib::close(again as usize);
}

fn print_args(out: &mut Fd, argc: usize, argv: *const *const u8) {
    let _ = writeln!(out, "argc={argc}");
    // SAFETY: as in `main`.
    for (i, a) in unsafe { userlib::args(argc, argv) }.enumerate() {
        let _ = writeln!(out, "argv[{i}]={a:?}");
    }
    // SAFETY: the kernel guarantees `argc + 1` readable pointers, the last a NULL.
    let terminator = unsafe { *argv.add(argc) };
    let sp: usize;
    // SAFETY: reads a register.
    unsafe { asm!("mov {}, sp", out(reg) sp) };
    let yes = |b: bool| if b { "yes" } else { "no" };
    let _ = writeln!(out, "argv[argc] is NULL: {}", yes(terminator.is_null()));
    let _ = writeln!(out, "argv is 16-byte aligned: {}", yes(argv as usize % 16 == 0));
    let _ = writeln!(out, "sp is 16-byte aligned: {}", yes(sp % 16 == 0));
}
