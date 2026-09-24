//! The shell's state that a script or a redirect must be able to save and restore: the working
//! directory and the three standard streams' bindings, held as a stack of frames. (Stage 16 adds the
//! environment to the frame; it becomes, in effect, the per-process state a child inherits.)
//!
//! Two different scoping operations, deliberately not one:
//! - `with_scope`: push a copy of the whole frame, run, pop. Everything the code inside changes -- the
//!   working directory, the stream bindings -- is gone afterwards. What a script run as its own process
//!   (`./script.sh`, `sh script.sh`) gets.
//! - `with_stdio`: replace some stream bindings on the *current* frame, run, put them back -- and nothing
//!   else. A `cd` done inside stays done. What every redirect uses, builtins included, so that
//!   `cd dir > f` still changes directory.
//!
//! A frame is plain data with no reference to any static, so it can be embedded in a per-process struct
//! unchanged when there is one. The stack itself is the one static (`shell_state.rs`).
//!
//! What an open file *is* is not this module's business: it is generic over the type `F` a binding holds
//! (`StdioBinding<F>`), which the kernel instantiates with a reference-counted open file
//! (`fs::files::FileRef`, see `shell_state.rs`) and the host tests with plain numbers or an `Rc`. Copying a
//! frame clones its bindings, so a scope inherits the same open files, and dropping the frame releases them.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use alloc::string::String;
use alloc::vec::Vec;

/// What standard stream `n` (0, 1 or 2) is connected to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdioBinding<F> {
    /// The default: the keyboard for stdin, the console for stdout and stderr.
    Default,
    /// An open file. Cloning the binding shares the file (a reference, for the kernel's `F`), so the file
    /// stays open until the last binding or fd that holds it is dropped.
    File(F),
}

/// One level of shell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellFrame<F> {
    /// The working directory: absolute and normalized (see `fs::path::abspath`).
    pub cwd: String,
    /// Where stdin, stdout and stderr go.
    pub stdio: [StdioBinding<F>; 3],
    // Stage 16 adds `env` here.
}

/// The stack of frames. Never empty: the bottom frame is the shell's own, and is never popped.
pub struct FrameStack<F> {
    frames: Vec<ShellFrame<F>>,
}

impl<F: Clone> FrameStack<F> {
    /// A stack holding just the shell's own frame: working directory `/`, default streams.
    pub fn new() -> Self {
        Self {
            frames: alloc::vec![ShellFrame {
                cwd: String::from("/"),
                stdio: [StdioBinding::Default, StdioBinding::Default, StdioBinding::Default],
            }],
        }
    }

    /// The frame code is currently running against.
    pub fn top(&self) -> &ShellFrame<F> {
        self.frames.last().expect("the frame stack is never empty")
    }

    pub fn top_mut(&mut self) -> &mut ShellFrame<F> {
        self.frames
            .last_mut()
            .expect("the frame stack is never empty")
    }

    /// How many frames there are; 1 is just the shell's own. `with_stdio` uses it to check it
    /// leaves the stack as it found it. (It does not limit script nesting:
    /// `shell::MAX_SCRIPT_DEPTH` is a separate counter, since it also has to catch *unscoped*
    /// recursion -- `source` sourcing itself -- which never pushes a frame at all.)
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Pushes a copy of the top frame.
    pub fn push_copy(&mut self) {
        let copy = self.top().clone();
        self.frames.push(copy);
    }

    /// Pops the top frame. Refuses (returns `false`) to pop the shell's own.
    pub fn pop(&mut self) -> bool {
        if self.frames.len() == 1 {
            return false;
        }
        self.frames.pop();
        true
    }

    /// Runs `f` against a copy of the current frame, then discards it: nothing `f` changes in the
    /// frame -- its `cd`s, its stream bindings -- survives.
    pub fn with_scope<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.push_copy();
        let result = f(self);
        self.pop();
        result
    }

    /// Runs `f` with the streams named in `overrides` (`Some(binding)` replaces stream `n`, `None` leaves
    /// it) rebound on the current frame, then puts the streams back. Only the streams are restored: a
    /// working-directory change made inside `f` is kept. The override bindings are dropped at the end, so
    /// a file opened for the redirect is closed then, unless something else still holds it.
    pub fn with_stdio<R>(
        &mut self,
        overrides: [Option<StdioBinding<F>>; 3],
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let depth = self.depth();
        let saved = self.top().stdio.clone();
        for (stream, binding) in self.top_mut().stdio.iter_mut().zip(overrides) {
            if let Some(binding) = binding {
                *stream = binding;
            }
        }
        let result = f(self);
        debug_assert_eq!(self.depth(), depth, "a frame was left pushed");
        self.top_mut().stdio = saved;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use StdioBinding::*;

    /// The tests' stand-in for an open file: a number.
    type FrameStack = super::FrameStack<u32>;

    #[test]
    fn it_starts_with_the_shells_own_frame() {
        let s = FrameStack::new();
        assert_eq!(s.depth(), 1);
        assert_eq!(s.top().cwd, "/");
        assert_eq!(s.top().stdio, [Default; 3]);
    }

    #[test]
    fn the_base_frame_cannot_be_popped() {
        let mut s = FrameStack::new();
        assert!(!s.pop());
        assert_eq!(s.depth(), 1);
        s.push_copy();
        assert!(s.pop());
        assert!(!s.pop());
    }

    #[test]
    fn a_scope_isolates_the_working_directory_and_the_streams() {
        let mut s = FrameStack::new();
        s.top_mut().cwd = String::from("/bin");
        s.with_scope(|s| {
            assert_eq!(s.top().cwd, "/bin"); // starts as a copy
            s.top_mut().cwd = String::from("/fonts");
            s.top_mut().stdio[1] = File(3);
        });
        assert_eq!(s.top().cwd, "/bin");
        assert_eq!(s.top().stdio, [Default; 3]);
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn scopes_nest() {
        let mut s = FrameStack::new();
        s.with_scope(|s| {
            s.top_mut().cwd = String::from("/a");
            s.with_scope(|s| {
                s.top_mut().cwd = String::from("/a/b");
                assert_eq!(s.depth(), 3);
            });
            assert_eq!(s.top().cwd, "/a"); // the inner one's change did not leak out
        });
        assert_eq!(s.top().cwd, "/");
    }

    #[test]
    fn with_stdio_restores_only_the_streams() {
        let mut s = FrameStack::new();
        s.with_stdio([None, Some(File(2)), None], |s| {
            assert_eq!(s.top().stdio, [Default, File(2), Default]);
            s.top_mut().cwd = String::from("/bin"); // a builtin `cd` under a redirect
        });
        assert_eq!(s.top().stdio, [Default; 3]);
        assert_eq!(s.top().cwd, "/bin"); // and it stuck
    }

    #[test]
    fn with_stdio_overrides_nest_and_unwind_in_order() {
        let mut s = FrameStack::new();
        s.with_stdio([None, Some(File(1)), None], |s| {
            s.with_stdio([None, Some(File(2)), Some(File(2))], |s| {
                assert_eq!(s.top().stdio, [Default, File(2), File(2)]);
            });
            assert_eq!(s.top().stdio, [Default, File(1), Default]);
        });
        assert_eq!(s.top().stdio, [Default; 3]);
    }

    #[test]
    fn a_scope_inside_a_redirect_and_a_redirect_inside_a_scope_both_unwind() {
        let mut s = FrameStack::new();
        s.with_stdio([None, Some(File(1)), None], |s| {
            s.with_scope(|s| {
                // The scope copied the redirected streams...
                assert_eq!(s.top().stdio[1], File(1));
                s.with_stdio([None, Some(File(9)), None], |s| {
                    assert_eq!(s.top().stdio[1], File(9));
                });
                assert_eq!(s.top().stdio[1], File(1));
                s.top_mut().cwd = String::from("/inner");
            });
            // ...and the scope's cd is gone, the outer redirect still in place.
            assert_eq!(s.top().cwd, "/");
            assert_eq!(s.top().stdio[1], File(1));
        });
        assert_eq!(s.top().stdio, [Default; 3]);
    }

    #[test]
    fn an_early_return_inside_the_closure_unbalances_nothing() {
        fn body(s: &mut FrameStack, fail: bool) -> Result<(), ()> {
            s.with_scope(|s| {
                s.with_stdio([Some(File(1)), None, None], |s| {
                    if fail {
                        return Err(());
                    }
                    s.top_mut().cwd = String::from("/x");
                    Ok(())
                })
            })
        }
        let mut s = FrameStack::new();
        assert_eq!(body(&mut s, true), Err(()));
        assert_eq!(body(&mut s, false), Ok(()));
        assert_eq!(
            (s.depth(), s.top().cwd.as_str(), s.top().stdio),
            (1, "/", [Default; 3])
        );
    }
    /// Bindings that count their holders, standing in for the kernel's reference-counted open files.
    mod shared {
        use super::super::{FrameStack, StdioBinding};
        use alloc::rc::Rc;

        #[test]
        fn a_scope_shares_the_files_and_releases_them_when_it_ends() {
            let file = Rc::new(());
            let mut s = FrameStack::new();
            s.top_mut().stdio[1] = StdioBinding::File(file.clone());
            assert_eq!(Rc::strong_count(&file), 2);
            s.with_scope(|s| {
                // The copy holds the same file, not a second one.
                assert_eq!(Rc::strong_count(&file), 3);
                s.top_mut().stdio[1] = StdioBinding::Default;
                assert_eq!(Rc::strong_count(&file), 2);
            });
            assert_eq!(Rc::strong_count(&file), 2);
            s.top_mut().stdio[1] = StdioBinding::Default;
            assert_eq!(Rc::strong_count(&file), 1);
        }

        #[test]
        fn with_stdio_releases_the_override_when_it_ends() {
            let file = Rc::new(());
            let mut s = FrameStack::new();
            s.with_stdio([None, Some(StdioBinding::File(file.clone())), None], |s| {
                assert_eq!(Rc::strong_count(&file), 2);
                // `2>&1`: another binding of the same file.
                let dup = s.top().stdio[1].clone();
                s.top_mut().stdio[2] = dup;
                assert_eq!(Rc::strong_count(&file), 3);
            });
            assert_eq!(Rc::strong_count(&file), 1);
        }

        #[test]
        fn a_redirect_layered_over_another_releases_the_one_it_replaces() {
            let pipe = Rc::new(());
            let mut s = FrameStack::new();
            s.with_stdio([None, Some(StdioBinding::File(pipe.clone())), None], |s| {
                s.top_mut().stdio[1] = StdioBinding::Default; // `cmd > elsewhere` replacing the pipe
                assert_eq!(Rc::strong_count(&pipe), 1);
            });
            assert_eq!(Rc::strong_count(&pipe), 1);
        }
    }
}
