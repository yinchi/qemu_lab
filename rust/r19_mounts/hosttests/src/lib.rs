//! Host-side tests for `r19_mounts`'s pure-logic modules. A module qualifies by being `no_std` +
//! `alloc` with no dependency on the rest of the kernel; each is pulled in here by path, and its
//! own `#[cfg(test)]` tests run as part of this crate.
//!
//! Modules are added as the Steps in `Stage12.md` create them.

#![allow(
    clippy::new_without_default,
    reason = "these modules are `pub` only so the tests can reach them; in the kernel they are private, where the lint does not apply"
)]

extern crate alloc;

#[path = "../../src/exec/argplan.rs"]
pub mod argplan;
#[path = "../../src/console/cells.rs"]
pub mod cells;
#[path = "../../src/exec/elfparse.rs"]
pub mod elfparse;
#[path = "../../src/console/font.rs"]
pub mod font;
#[path = "../../src/exec/frame_stack.rs"]
pub mod frame_stack;

/// The kernel names its modules by their directory (`crate::exec::frame_stack`); here they are all at the top, so
/// this alias lets a file that reaches across directories (`shell/environment.rs`) compile unchanged in both.
pub mod exec {
    pub use crate::frame_stack;
}
#[path = "../../src/keyboard/history.rs"]
pub mod history;
#[path = "../../src/console/input_layout.rs"]
pub mod input_layout;
// `line.rs` uses `super::tokens`, which in turn uses `super::keymap` (for `Token::char()`'s
// `KEY_NAMES` lookup, only exercised by tests that hold Ctrl -- see `line.rs`'s `feed` doc
// comment) and `crate::static_ref!` (from `util.rs`) -- pulled in below so the whole module tree
// still compiles here, not because these tests populate `KEY_NAMES` themselves.
#[path = "../../src/keyboard/keymap.rs"]
pub mod keymap;
#[path = "../../src/shell/environment.rs"]
pub mod environment;
#[path = "../../src/shell/expand.rs"]
pub mod expand;
#[path = "../../src/shell/lexer.rs"]
pub mod lexer;
#[path = "../../src/shell/path_search.rs"]
pub mod path_search;
#[path = "../../src/shell/prompt.rs"]
pub mod prompt;
#[path = "../../src/keyboard/line.rs"]
pub mod line;
#[path = "../../src/fs/bootsector.rs"]
pub mod bootsector;
#[path = "../../src/fs/fattime.rs"]
pub mod fattime;
#[path = "../../src/fs/mounttable.rs"]
pub mod mounttable;

/// As `exec` above: `fs/mounttable.rs`'s tests name `crate::fs::bootsector`.
pub mod fs {
    pub use crate::bootsector;
}
#[path = "../../src/fs/path.rs"]
pub mod path;
#[path = "../../src/keyboard/ring_buffer.rs"]
pub mod ring_buffer;
#[path = "../../src/shell/syntax.rs"]
pub mod syntax;
#[path = "../../src/keyboard/tokens.rs"]
pub mod tokens;
#[path = "../../src/exec/usermem.rs"]
pub mod usermem;
#[path = "../../src/console/utf8.rs"]
pub mod utf8;
#[path = "../../src/util.rs"]
pub mod util;
#[path = "../../../user/progs_r18/src/human.rs"]
pub mod human;
#[path = "../../../user/progs_r18/src/cutlist.rs"]
pub mod cutlist;
#[path = "../../../user/progs_r18/src/glob.rs"]
pub mod glob;
#[path = "../../../user/progs_r18/src/sortkey.rs"]
pub mod sortkey;
#[path = "../../../user/progs_r18/src/textutil.rs"]
pub mod textutil;
#[path = "../../../user/progs_r18/src/trset.rs"]
pub mod trset;
#[path = "../../../user/progs_r18/src/countspec.rs"]
pub mod countspec;
#[path = "../../../user/progs_r19/src/spell.rs"]
pub mod spell;
#[path = "../../../user/progs_r19/src/table.rs"]
pub mod table;
