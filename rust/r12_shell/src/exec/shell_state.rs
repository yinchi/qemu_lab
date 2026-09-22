//! The shell's state as the rest of the kernel sees it: the one `FrameStack` (`frame_stack.rs`), and the
//! operations on it that other layers need -- the working directory, path resolution against it, and
//! `chdir`.
//!
//! It lives in `exec/`, not `shell/`, because it is read from below the shell: `syscall/` resolves a
//! program's relative paths against the working directory and starts each program with the top frame's
//! stream bindings, while `shell/` is what changes them (`cd`, and later scopes and redirects). There is
//! one stack, not one per program: at most one program is ever resident, so the shell's state *is* the
//! running program's state (Stage 19+ revisits this).
//!
//! There is deliberately no `chdir` *syscall* yet: with one global stack a program's `chdir` would change
//! the shell's directory too, and real Unix's per-process isolation doesn't exist until state is
//! per-process. Its number is reserved in `abi::syscall`.

use alloc::string::String;

use super::frame_stack::{FrameStack, StdioBinding};
use crate::fs::{files, path};
use crate::static_mut_ref;

/// Written once by `kernel_main`, before the keyboard's interrupt is enabled.
/// SAFETY (every access): single core; the shell's loop and the syscalls it runs never overlap, and the
/// interrupt handler never touches it.
pub static mut FRAMES: Option<FrameStack> = None;

/// The stack of shell frames.
pub fn frames() -> &'static mut FrameStack {
    // SAFETY: see FRAMES; populated by `kernel_main` before anything can call this.
    unsafe { static_mut_ref!(FRAMES) }
}

/// The working directory, as an absolute path.
pub fn cwd() -> String {
    frames().top().cwd.clone()
}

/// The absolute path `path` names from the working directory (see `path::abspath`).
pub fn absolute(path: &str) -> Result<String, isize> {
    path::abspath(&frames().top().cwd, path)
}

/// Makes `path` (relative to the working directory) the working directory, if it names a directory.
/// On error the working directory is unchanged.
pub fn chdir(path: &str) -> Result<(), isize> {
    let target = absolute(path)?;
    files::check_directory(&target)?;
    frames().top_mut().cwd = target;
    Ok(())
}

/// Where standard stream `n` goes for a program about to start.
pub fn stdio(n: usize) -> StdioBinding {
    frames().top().stdio[n]
}
