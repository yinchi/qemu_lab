//! The program break: `brk`, the one call a program uses to get more memory as it runs. `heap.rs` (behind
//! the `heap` feature) is the allocator built on it; a program that manages its own memory can call `brk`
//! directly.

use abi::syscall::SYS_BRK;

/// Moves the program break to `addr` and returns the break the kernel ended up with. `brk(0)` just returns
/// the current one. Not an errno convention -- Linux's, kept: a request the kernel cannot grant leaves the
/// break where it was and returns *that*, so a caller checks `brk(want) == want`. The heap starts at the
/// end of the program's image, and everything between there and the break is zeroed and writable.
pub fn brk(addr: usize) -> usize {
    syscall!(SYS_BRK, addr) as usize
}
