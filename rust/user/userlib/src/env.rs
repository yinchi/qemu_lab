//! A program's environment: the `NAME=VALUE` strings the kernel put on its stack (`envp`, the third
//! argument `_start` receives in `x2`). A program that wants it starts with `entry_with_env!`, which
//! records the pointer here; `var` and `vars` then read it. A program started with `entry!` or
//! `entry_with_args!` never records one, and sees an empty environment -- so a binary built for a kernel
//! older than Stage 17, which puts nothing meaningful in `x2`, is unaffected.

use core::sync::atomic::{AtomicUsize, Ordering};

/// The `envp` array recorded by `entry_with_env!`, or 0 if it never ran.
static ENVP: AtomicUsize = AtomicUsize::new(0);

/// Records `envp` for `var` and `vars`. Called by the `entry_with_env!`-generated `main`, not by
/// programs.
///
/// # Safety
/// `envp` must point to a `NULL`-terminated array of pointers to NUL-terminated, UTF-8 strings, all
/// still mapped for the rest of the program -- true of whatever `_start` forwards from the kernel's
/// own `eret`, never something a program should construct itself.
#[doc(hidden)]
pub unsafe fn set_envp(envp: *const *const u8) {
    ENVP.store(envp as usize, Ordering::Relaxed);
}

/// The environment as `(name, value)` pairs, in the order the shell exported them.
#[derive(Clone, Copy)]
pub struct Vars {
    next: *const *const u8,
}

impl Iterator for Vars {
    type Item = (&'static str, &'static str);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.next.is_null() {
                return None;
            }
            // SAFETY: `set_envp`'s contract -- `next` points into a NULL-terminated array of valid
            // NUL-terminated strings, and never moves past the terminator.
            let entry = unsafe {
                let ptr = *self.next;
                if ptr.is_null() {
                    return None;
                }
                self.next = self.next.add(1);
                let mut len = 0;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                core::slice::from_raw_parts(ptr, len)
            };
            // The kernel writes only `NAME=VALUE` strings built from its own `str`s; anything else
            // would mean that contract was already broken, so it is skipped rather than guessed at.
            if let Ok(text) = core::str::from_utf8(entry)
                && let Some(pair) = text.split_once('=')
            {
                return Some(pair);
            }
        }
    }
}

/// Every variable in this program's environment.
pub fn vars() -> Vars {
    Vars { next: ENVP.load(Ordering::Relaxed) as *const *const u8 }
}

/// The value of the variable `name`, or `None` if the environment has no such variable.
pub fn var(name: &str) -> Option<&'static str> {
    vars().find(|(n, _)| *n == name).map(|(_, value)| value)
}
