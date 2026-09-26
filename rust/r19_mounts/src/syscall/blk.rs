//! The `blkinfo` syscall: what the kernel knows about each block device (`fs/devices.rs`).

use abi::blk::BLKINFO_SIZE;
use abi::errno::{EFAULT, ENODEV};

use super::fd::validate;
use crate::fs::devices;

/// Writes the description of block device `index` (`abi::blk`'s 32-byte record) to the user buffer `ptr`:
/// `ENODEV` past the last device, `EFAULT` if `ptr` is not writable user memory.
pub fn blkinfo(index: usize, ptr: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // writes a user buffer: clear PAN while it does
    let Some(info) = devices::blkinfo(index) else {
        return ENODEV;
    };
    if !validate(ptr, BLKINFO_SIZE, true) {
        return EFAULT;
    }
    // SAFETY: validated above to lie entirely within writable user memory.
    let out = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, BLKINFO_SIZE) };
    out.copy_from_slice(&info.encode());
    0
}
