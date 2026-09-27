//! The shell's state that a script or a redirect must be able to save and restore: the working
//! directory, the three standard streams' bindings and the shell's variables, held as a stack of frames. It is,
//! in effect, the per-process state a child inherits.
//!
//! **Variables** are POSIX's: each has a name, a value and an `exported` flag. All of them can be read (`$NAME`);
//! only the exported ones are handed on to a program (its `envp`) and to a script run as its own process.
//! Assigning keeps a variable's flag (a new one is not exported); `export` sets the flag; `unset` removes.
//! `NAME=value command` gives the command a variable for as long as it runs: the shell sets it exported, and
//! puts it back afterwards with `saved_var` and `restore_var`.
//!
//! Three different scoping operations, deliberately not one:
//! - `with_scope`: push a *child process's* frame, run, pop. It starts with the working directory and stream
//!   bindings of the frame it was pushed on, but only the **exported** variables, all of them exported, as a real
//!   child process would inherit them. Everything the code inside changes -- the working directory, the stream
//!   bindings, the variables -- is gone afterwards. What a script run as its own process (`./script.sh`,
//!   `sh script.sh`) gets. (`source` pushes nothing: it runs against the current frame.)
//! - `with_subshell`: push a *forked* frame -- the top frame copied whole, every variable with its exported flag
//!   as well as the working directory and streams -- run, pop. What a stage of a pipeline gets, as in a POSIX shell
//!   where each stage is a subshell: it can read the shell's unexported variables, and nothing it changes
//!   survives. Unlike `with_scope`, which models an *exec* (only the environment crosses).
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

/// A shell variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub name: String,
    pub value: String,
    /// Whether programs the shell starts (and scripts run as their own process) receive it.
    pub exported: bool,
}

/// Whether `name` is a valid variable name: a letter or `_`, then letters, digits and `_`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The name given to a variable operation was not a valid name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidName;

/// One level of shell state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellFrame<F> {
    /// The working directory: absolute and normalized (see `fs::path::abspath`).
    pub cwd: String,
    /// Where stdin, stdout and stderr go.
    pub stdio: [StdioBinding<F>; 3],
    /// The shell's variables, in the order they were first set.
    pub vars: Vec<Var>,
}

// `is_exported` is used by the tests only; the kernel reads a variable's flag through `exported()`.
#[allow(dead_code)]
impl<F> ShellFrame<F> {
    /// The value of the variable `name`, if it is set (exported or not).
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.iter().find(|v| v.name == name).map(|v| v.value.as_str())
    }

    /// Whether `name` is set and exported.
    pub fn is_exported(&self, name: &str) -> bool {
        self.vars.iter().any(|v| v.name == name && v.exported)
    }

    /// Sets `name` to `value`. An existing variable keeps its exported flag and its place in the order; a new
    /// one is not exported.
    pub fn set_var(&mut self, name: &str, value: &str) -> Result<(), InvalidName> {
        if !is_valid_name(name) {
            return Err(InvalidName);
        }
        match self.vars.iter_mut().find(|v| v.name == name) {
            Some(v) => v.value = String::from(value),
            None => self.vars.push(Var {
                name: String::from(name),
                value: String::from(value),
                exported: false,
            }),
        }
        Ok(())
    }

    /// `export NAME[=VALUE]`: with a value, sets the variable and exports it; without one, exports it if it is
    /// set and does nothing if it is not (there is nothing to mark).
    pub fn export_var(&mut self, name: &str, value: Option<&str>) -> Result<(), InvalidName> {
        if !is_valid_name(name) {
            return Err(InvalidName);
        }
        if let Some(value) = value {
            self.set_var(name, value)?;
        }
        if let Some(v) = self.vars.iter_mut().find(|v| v.name == name) {
            v.exported = true;
        }
        Ok(())
    }

    /// What `name` is now -- its value and whether it is exported -- or `None` if it is not set: what
    /// `restore_var` needs to put it back.
    pub fn saved_var(&self, name: &str) -> Option<(String, bool)> {
        self.vars.iter().find(|v| v.name == name).map(|v| (v.value.clone(), v.exported))
    }

    /// Puts `name` back as `saved_var` found it: the old value and flag, or not set at all. Whatever happened
    /// to the variable in between (changed, unset) is undone; one that was set before keeps its place in the
    /// order if it is still there, and otherwise goes to the end.
    pub fn restore_var(&mut self, name: &str, saved: Option<(String, bool)>) {
        let Some((value, exported)) = saved else {
            self.unset_var(name);
            return;
        };
        match self.vars.iter_mut().find(|v| v.name == name) {
            Some(v) => {
                v.value = value;
                v.exported = exported;
            }
            None => self.vars.push(Var { name: String::from(name), value, exported }),
        }
    }

    /// Removes `name`, whether or not it was set.
    pub fn unset_var(&mut self, name: &str) {
        self.vars.retain(|v| v.name != name);
    }

    /// The exported variables as `(name, value)`, in order: what a program gets as its environment.
    pub fn exported(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().filter(|v| v.exported).map(|v| (v.name.as_str(), v.value.as_str()))
    }
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
                vars: Vec::new(),
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

    /// The working directory of every frame, innermost last: what a `umount` must find none of inside the volume.
    pub fn cwds(&self) -> impl Iterator<Item = &str> {
        self.frames.iter().map(|f| f.cwd.as_str())
    }

    /// How many frames there are; 1 is just the shell's own. `with_stdio` uses it to check it
    /// leaves the stack as it found it. (It does not limit script nesting:
    /// `shell::MAX_SCRIPT_DEPTH` is a separate counter, since it also has to catch *unscoped*
    /// recursion -- `source` sourcing itself -- which never pushes a frame at all.)
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Pushes the frame a child process would start with: the top frame's working directory and stream
    /// bindings, and only its exported variables (all of them exported: they are the child's environment).
    pub fn push_child(&mut self) {
        let top = self.top();
        let child = ShellFrame {
            cwd: top.cwd.clone(),
            stdio: top.stdio.clone(),
            vars: top
                .vars
                .iter()
                .filter(|v| v.exported)
                .cloned()
                .collect(),
        };
        self.frames.push(child);
    }

    /// Pushes the frame a forked subshell would start with: a full copy of the top frame.
    pub fn push_subshell(&mut self) {
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

    /// Runs `f` against a child process's frame (`push_child`), then discards it: nothing `f` changes in the
    /// frame -- its `cd`s, its stream bindings, its variables -- survives.
    pub fn with_scope<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.push_child();
        let result = f(self);
        self.pop();
        result
    }

    /// Runs `f` against a forked subshell's frame (`push_subshell`), then discards it, as `with_scope` does for a
    /// child process. The difference is what `f` starts with: everything, unexported variables included.
    pub fn with_subshell<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.push_subshell();
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
    fn cwds_lists_every_frames_working_directory_innermost_last() {
        let mut s = FrameStack::new();
        s.top_mut().cwd = String::from("/a");
        s.push_child();
        s.top_mut().cwd = String::from("/b");
        assert_eq!(s.cwds().collect::<Vec<_>>(), ["/a", "/b"]);
        s.pop();
        assert_eq!(s.cwds().collect::<Vec<_>>(), ["/a"]);
    }

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
        s.push_child();
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
    fn names_are_letters_digits_and_underscores_not_starting_with_a_digit() {
        for good in ["A", "_", "_x", "HOME", "a1", "TZ2_b"] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in ["", "1a", "a-b", "a b", "a=b", "é", "$X"] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn setting_a_variable_keeps_its_place_and_its_exported_flag() {
        let mut f = FrameStack::new();
        let top = f.top_mut();
        top.set_var("A", "1").unwrap();
        top.set_var("B", "2").unwrap();
        top.export_var("A", None).unwrap();
        top.set_var("A", "one").unwrap(); // replaced: still exported, still first
        assert_eq!(top.var("A"), Some("one"));
        assert!(top.is_exported("A") && !top.is_exported("B"));
        assert_eq!(top.vars.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(top.var("C"), None);
    }

    #[test]
    fn export_marks_assigns_and_ignores_the_unset() {
        let mut f = FrameStack::new();
        let top = f.top_mut();
        top.export_var("X", Some("1")).unwrap(); // assign and export
        top.export_var("NOSUCH", None).unwrap(); // nothing to mark
        assert_eq!(top.var("NOSUCH"), None);
        top.set_var("Y", "2").unwrap();
        assert!(!top.is_exported("Y"));
        top.export_var("Y", None).unwrap();
        assert_eq!(top.exported().collect::<Vec<_>>(), [("X", "1"), ("Y", "2")]);
        top.unset_var("X");
        top.unset_var("NOSUCH"); // harmless
        assert_eq!(top.exported().collect::<Vec<_>>(), [("Y", "2")]);
    }

    #[test]
    fn a_bad_name_is_refused_by_every_setter() {
        let mut f = FrameStack::new();
        let top = f.top_mut();
        assert_eq!(top.set_var("a-b", "x"), Err(InvalidName));
        assert_eq!(top.export_var("1a", Some("x")), Err(InvalidName));
        assert_eq!(top.export_var("", None), Err(InvalidName));
        assert!(top.vars.is_empty());
    }

    #[test]
    fn a_child_scope_inherits_only_exported_variables_and_loses_its_changes() {
        let mut f = FrameStack::new();
        f.top_mut().export_var("OUT", Some("visible")).unwrap();
        f.top_mut().set_var("LOCAL", "hidden").unwrap();
        f.with_scope(|s| {
            assert_eq!(s.top().var("OUT"), Some("visible"));
            assert_eq!(s.top().var("LOCAL"), None); // not exported: a child never sees it
            assert!(s.top().is_exported("OUT")); // and what it got is its own environment
            s.top_mut().set_var("OUT", "changed").unwrap();
            s.top_mut().set_var("NEW", "made").unwrap();
        });
        assert_eq!(f.top().var("OUT"), Some("visible")); // nothing leaked out
        assert_eq!(f.top().var("NEW"), None);
        assert_eq!(f.top().var("LOCAL"), Some("hidden")); // and nothing was lost
    }

    #[test]
    fn a_shared_frame_sees_everything_and_keeps_changes_as_source_does() {
        let mut f = FrameStack::new();
        f.top_mut().set_var("LOCAL", "hidden").unwrap();
        // `source` pushes nothing: it works on the current frame.
        assert_eq!(f.top().var("LOCAL"), Some("hidden"));
        f.top_mut().set_var("MADE", "1").unwrap();
        assert_eq!(f.top().var("MADE"), Some("1"));
    }

    #[test]
    fn variables_survive_a_redirect_scope() {
        let mut f = FrameStack::new();
        f.with_stdio([None, Some(File(1)), None], |s| {
            s.top_mut().set_var("V", "set inside").unwrap();
        });
        assert_eq!(f.top().var("V"), Some("set inside")); // like a `cd` under a redirect
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

    #[test]
    fn a_variable_can_be_saved_and_put_back() {
        let mut stack = FrameStack::new();
        let f = stack.top_mut();
        f.set_var("KEEP", "old").unwrap();
        f.export_var("EXP", Some("e")).unwrap();
        f.set_var("LAST", "l").unwrap();
        let saved: Vec<_> = ["KEEP", "EXP", "NEW"].iter().map(|n| (*n, f.saved_var(n))).collect();
        assert_eq!(saved[0].1, Some(("old".into(), false)));
        assert_eq!(saved[1].1, Some(("e".into(), true)));
        assert_eq!(saved[2].1, None);
        // The overlay: all three set and exported, with new values.
        for n in ["KEEP", "EXP", "NEW"] {
            f.export_var(n, Some("tmp")).unwrap();
        }
        assert_eq!(f.exported().count(), 3);
        for (n, s) in saved {
            f.restore_var(n, s);
        }
        assert_eq!(f.var("KEEP"), Some("old"));
        assert!(!f.is_exported("KEEP"), "it was not exported before, so it is not now");
        assert!(f.is_exported("EXP"));
        assert_eq!(f.var("EXP"), Some("e"));
        assert_eq!(f.var("NEW"), None);
        let names: Vec<_> = f.vars.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["KEEP", "EXP", "LAST"], "order is as it was");
    }

    #[test]
    fn restoring_undoes_what_the_command_did() {
        let mut stack = FrameStack::new();
        let f = stack.top_mut();
        f.export_var("A", Some("1")).unwrap();
        let saved = f.saved_var("A");
        f.unset_var("A"); // the command unset it
        f.set_var("B", "x").unwrap();
        f.restore_var("A", saved);
        assert_eq!((f.var("A"), f.is_exported("A")), (Some("1"), true));
        assert_eq!(f.var("B"), Some("x"), "other variables are left alone");
        // Restoring "not set" removes what the command made.
        f.restore_var("B", None);
        assert_eq!(f.var("B"), None);
    }

    #[test]
    fn a_subshell_starts_as_a_full_copy() {
        let mut s = FrameStack::new();
        s.top_mut().cwd = String::from("/tests");
        s.top_mut().stdio[1] = File(7);
        s.top_mut().set_var("PLAIN", "p").unwrap();
        s.top_mut().export_var("EXPORTED", Some("e")).unwrap();
        s.with_subshell(|s| {
            assert_eq!(s.depth(), 2);
            assert_eq!(s.top().cwd, "/tests");
            assert_eq!(s.top().stdio, [Default, File(7), Default]);
            // Unlike a child process's, it has the variable that was never exported, and flags are kept.
            assert_eq!(s.top().var("PLAIN"), Some("p"));
            assert!(!s.top().is_exported("PLAIN"));
            assert!(s.top().is_exported("EXPORTED"));
        });
        assert_eq!(s.with_scope(|s| s.top().var("PLAIN").map(String::from)), None);
    }

    #[test]
    fn nothing_a_subshell_does_survives() {
        let mut s = FrameStack::new();
        s.top_mut().set_var("KEEP", "k").unwrap();
        s.top_mut().export_var("GONE", Some("g")).unwrap();
        s.with_subshell(|s| {
            s.top_mut().cwd = String::from("/elsewhere");
            s.top_mut().stdio[2] = File(9);
            s.top_mut().unset_var("KEEP");
            s.top_mut().unset_var("GONE");
            s.top_mut().set_var("NEW", "n").unwrap();
            s.top_mut().export_var("KEEP2", Some("x")).unwrap();
        });
        assert_eq!(s.depth(), 1);
        assert_eq!(s.top().cwd, "/");
        assert_eq!(s.top().stdio, [Default; 3]);
        assert_eq!(s.top().var("KEEP"), Some("k"));
        assert_eq!(s.top().var("GONE"), Some("g"));
        assert_eq!(s.top().var("NEW"), None);
        assert_eq!(s.top().var("KEEP2"), None);
    }

    #[test]
    fn subshells_nest_with_the_other_scopes() {
        let mut s = FrameStack::new();
        s.top_mut().set_var("A", "1").unwrap();
        s.with_subshell(|s| {
            s.top_mut().set_var("A", "2").unwrap();
            s.with_stdio([Some(File(3)), None, None], |s| {
                s.with_subshell(|s| {
                    assert_eq!(s.depth(), 3);
                    assert_eq!(s.top().var("A"), Some("2"));
                    assert_eq!(s.top().stdio[0], File(3));
                });
            });
            assert_eq!(s.top().stdio[0], Default);
            assert_eq!(s.top().var("A"), Some("2"));
        });
        assert_eq!(s.top().var("A"), Some("1"));
    }
}
