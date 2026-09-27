//! The block-device syscalls: `blkinfo` (what the kernel knows about each device, `fs/devices.rs`) and `mount`/`umount`
//! (`fs/mounts.rs`).

use abi::blk::BLKINFO_SIZE;
use abi::errno::{EFAULT, ENODEV};

use super::fd::{user_path, validate};
use crate::exec::shell_state;
use crate::fs::{devices, mounts};

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

/// Mounts the volume the user string `src_ptr`/`src_len` names (`LABEL=..`/`UUID=..`) on the directory
/// `dst_ptr`/`dst_len`, a path taken from the working directory like any other -- see `mounts::mount`.
pub fn mount(src_ptr: usize, src_len: usize, dst_ptr: usize, dst_len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let source = match user_path(src_ptr, src_len) {
        Ok(source) => source,
        Err(e) => return e,
    };
    let target = match user_path(dst_ptr, dst_len).and_then(shell_state::absolute) {
        Ok(target) => target,
        Err(e) => return e,
    };
    mounts::mount(source, &target)
}

/// Unmounts the volume mounted at the user path `ptr`/`len` -- see `mounts::umount`.
pub fn umount(ptr: usize, len: usize) -> isize {
    let _user = crate::arch::mmu::user_access(); // these touch a user pointer: clear PAN while they do
    let target = match user_path(ptr, len).and_then(shell_state::absolute) {
        Ok(target) => target,
        Err(e) => return e,
    };
    mounts::umount(&target)
}
