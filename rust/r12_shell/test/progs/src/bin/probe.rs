//! `probe` -- a test-only program that pokes at the kernel's syscall surface from EL0, so the
//! harness can check what the kernel does with input a well-behaved program never sends. Each Step of
//! `Stage12.md` that needs a new probe adds a subcommand here; see `test/README.md`.
//!
//!   probe sys-unknown   an unassigned syscall number
//!   probe bad-ptr       bad or wrapping pointers/lengths given to write/read/open/chmod
//!   probe fds           opens files until the kernel says no, then closes them all
//!   probe args ...      prints argc/argv exactly as received, plus what the stack layout guarantees
//!   probe exit N        exits with status N, passed to the kernel unmasked (so N > 255 tests the mask)
//!   probe frag          one line of 200 one-digit `write!` fragments (stdout buffer: one console flush)
//!   probe frag-raw      the same line as 200 raw `write` syscalls (one console flush each)
//!   probe poke ADDR     reads one byte at ADDR (decimal) and prints it; a bad address faults (exit 139)
//!   probe poke-w ADDR   writes one byte at ADDR, reads it back, prints it
//!   probe user-ptrs    syscalls given pointers into the guard, the gap, read-only code and past the stack:
//!                      each must be refused (-14), none may fault the kernel
//!   probe sp            prints the stack pointer `main` runs with
//!   probe stack KIB     legitimately uses KIB KiB of stack (recursion, one KiB per frame) and prints a checksum
//!   probe bs-wide       a wide glyph, backspace, then `X`: `X` must land on the glyph's left cell
//!   probe interleave    `OUT` (no newline) to stdout, `ERR` to stderr, then a newline to stdout

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
        Some("frag") => {
            for i in 0..200 {
                let _ = write!(out, "{}", i % 10);
            }
            let _ = writeln!(out);
            0
        }
        Some("frag-raw") => {
            for i in 0..200 {
                userlib::write(1, &[b'0' + (i % 10) as u8]);
            }
            userlib::write(1, b"\n");
            0
        }
        Some("poke") => match args.next().and_then(progs::atoi) {
            Some(addr) => {
                // SAFETY: none -- the point is to touch an address the kernel may not have mapped.
                let byte = unsafe { core::ptr::read_volatile(addr as *const u8) };
                let _ = writeln!(out, "read {addr:#x}: {byte:#x}");
                0
            }
            None => usage_exit("probe poke ADDR"),
        },
        Some("poke-w") => match args.next().and_then(progs::atoi) {
            Some(addr) => {
                // SAFETY: as above.
                let byte = unsafe {
                    core::ptr::write_volatile(addr as *mut u8, 0xAA);
                    core::ptr::read_volatile(addr as *const u8)
                };
                let _ = writeln!(out, "wrote {addr:#x}: {byte:#x}");
                0
            }
            None => usage_exit("probe poke-w ADDR"),
        },
        Some("user-ptrs") => {
            user_ptrs(&mut out);
            0
        }
        Some("sp") => {
            let sp: usize;
            // SAFETY: reads a register.
            unsafe { asm!("mov {}, sp", out(reg) sp) };
            let _ = writeln!(out, "sp {sp:#x}");
            0
        }
        Some("stack") => match args.next().and_then(progs::atoi) {
            Some(kib) => {
                let _ = writeln!(out, "stack {kib} KiB: {}", use_stack(kib));
                0
            }
            None => usage_exit("probe stack KIB"),
        },
        Some("bs-wide") => {
            let _ = write!(out, "日\u{8}X\n");
            0
        }
        Some("interleave") => {
            let _ = write!(out, "OUT");
            let _ = write!(Fd(2), "ERR");
            let _ = writeln!(out);
            0
        }
        Some("exit") => match args.next().and_then(progs::atoi) {
            Some(n) => n as i32,
            None => {
                let _ = writeln!(Fd(2), "usage: probe exit N");
                2
            }
        },
        _ => {
            let _ = writeln!(Fd(2), "usage: probe sys-unknown|bad-ptr|fds|args|exit|poke|poke-w|user-ptrs|sp|stack|frag|frag-raw|bs-wide|interleave ...");
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

/// User-window addresses (see the kernel's `platform/base_addresses.rs`) that are not backed by memory
/// the kernel may write, or at all.
fn user_ptrs(out: &mut Fd) {
    const BASE: usize = 0x4400_0000;
    const GAP: usize = BASE + 0x8_0000; // between the program image and the guard
    const GUARD: usize = BASE + 0xF_0000;
    const STACK_TOP: usize = BASE + 0x20_0000;
    let code = user_ptrs as *const () as usize; // this program's own (read-only) code
    // A file to read from, so `read` has a real fd that returns at once.
    let fd = userlib::open("tests/hello.txt", userlib::O_RDONLY);
    let dir = userlib::open("tests", userlib::O_RDONLY);
    let cases: [(&str, isize); 9] = [
        ("write from the gap", raw(SYS_WRITE, 1, GAP, 16, 0)),
        ("write from the guard", raw(SYS_WRITE, 1, GUARD, 16, 0)),
        ("write running off the top of the stack", raw(SYS_WRITE, 1, STACK_TOP - 8, 16, 0)),
        ("read into read-only code", raw(SYS_READ, fd as usize, code, 16, 0)),
        ("read into the guard", raw(SYS_READ, fd as usize, GUARD, 16, 0)),
        ("getdents into read-only code", raw(SYS_GETDENTS, dir as usize, code, 64, 0)),
        ("open with the path in the guard", raw(SYS_OPEN, GUARD, 5, 0, 0)),
        ("chmod with the path in the gap", raw(SYS_CHMOD, GAP, 4, 0, 0)),
        ("write from code (the kernel only reads it: allowed)", raw(SYS_WRITE, 1, code, 0, 0)),
    ];
    for (what, ret) in cases {
        let _ = writeln!(out, "{what}: {ret}");
    }
}

fn usage_exit(text: &str) -> i32 {
    let _ = writeln!(Fd(2), "usage: {text}");
    2
}

/// Recurses `kib` frames of about 1 KiB each, touching every one, so exactly that much stack is
/// really used (not optimized away). Returns a checksum of what it wrote.
#[inline(never)]
fn use_stack(kib: usize) -> usize {
    let mut frame = [0u8; 1024];
    for (i, byte) in frame.iter_mut().enumerate() {
        // SAFETY: a plain write to our own array; volatile so the frame is not optimized away.
        unsafe { core::ptr::write_volatile(byte, (i ^ kib) as u8) };
    }
    let below = if kib > 1 { use_stack(kib - 1) } else { 0 };
    // SAFETY: as above.
    below + unsafe { core::ptr::read_volatile(&frame[kib % 1024]) } as usize
}
