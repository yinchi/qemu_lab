//! The text of the line being typed, and where the cursor sits within it (a byte offset, always on
//! a char boundary): Stage 5's cursor-aware editing model (`r05_lineedit/src/main.rs`), adapted from
//! raw ANSI redraws to screen-cell addressing for Step 12 of `Stage12.md`. This is deliberately the
//! *only* place Backspace/Delete/cursor movement ever get handled -- absorbed here, before Enter is
//! ever reached, not forwarded as raw control bytes to anything else. The line discipline
//! (`line_discipline.rs`) owns one and is the only caller.
//!
//! Deliberately a separate module from `tokens.rs`, same reasoning as that module's own
//! separation from `keymap.rs`: this is one particular interpretation of a `Token` stream (an
//! editable line, stopping at Enter), not the only one a future raw-mode consumer might want.

use alloc::string::String;

use super::tokens::{
    KEY_A, KEY_BACKSPACE, KEY_DELETE, KEY_E, KEY_END, KEY_ENTER, KEY_HOME, KEY_K, KEY_LEFT,
    KEY_RIGHT, KEY_U, Token,
};

/// How a line is being read -- defined here (not `line_discipline.rs`) so `LineBuffer::feed`'s mode
/// gating stays a dependency of pure-logic `line.rs` alone, not of the kernel-dependent line
/// discipline; `line_discipline.rs` re-exports this rather than defining its own copy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The shell's prompt: Ctrl+D does nothing (the shell is init and never exits on end-of-file).
    Prompt,
    /// A program's `read(0)`, a tty's canonical mode: Ctrl+D on an empty line is end-of-file.
    Canonical,
}

/// What feeding one `Token` into the buffer did -- as `Option<LineEvent>` to include the
/// possibility of no state change (an unbound key, or a no-op like Left at column 0).
pub enum LineEvent {
    /// The buffer's visible text changed (an insert, Backspace, Delete, or a kill) -- redraw the line.
    Changed,
    /// Only the cursor moved; the text didn't -- redraw just the two affected cells.
    CursorMoved,
    /// Enter finished the line: here's what was typed; the buffer is now empty again.
    Finished(String),
}

/// A line-editing buffer with an insertion point (`cursor`, a byte offset into `text`, always kept
/// on a char boundary).
pub struct LineBuffer {
    text: String,
    cursor: usize,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    /// The buffer's current content, for display.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The cursor's current byte offset, for the line discipline's redraw math.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether nothing has been typed on the current line.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces the buffer's content wholesale -- history recall (`line_discipline.rs`'s Up/Down
    /// handling). The cursor goes to the end, as on a freshly-typed line.
    pub fn set(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.cursor = self.text.len();
    }

    /// Clears the buffer and returns what was in it. Shared by Enter and by Ctrl+D's
    /// partial-delivery on a non-empty line (`line_discipline.rs`), rather than each
    /// reimplementing "take the text and reset the cursor."
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        core::mem::take(&mut self.text)
    }

    /// The byte offset of the character boundary immediately before the cursor, or `None` if the
    /// cursor is already at the start.
    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    /// The byte offset of the character boundary immediately after the cursor, or `None` if the
    /// cursor is already at the end.
    fn next_boundary(&self) -> Option<usize> {
        let c = self.text[self.cursor..].chars().next()?;
        Some(self.cursor + c.len_utf8())
    }

    /// Inserts `c` at the cursor and advances past it.
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Removes the character immediately before the cursor, if any. `pub(super)`, not private: also
    /// used by `line_discipline.rs` to undo an insert that would overflow the screen (an insert is
    /// always the most recent edit at that point, so undoing it is exactly "remove the character
    /// before the cursor").
    pub(super) fn backspace(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.text.remove(start);
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    /// Removes the character at/after the cursor, if any -- the cursor itself doesn't move.
    fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.text.len() {
            return false;
        }
        self.text.remove(self.cursor);
        true
    }

    fn move_left(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    fn move_right(&mut self) -> bool {
        match self.next_boundary() {
            Some(end) => {
                self.cursor = end;
                true
            }
            None => false,
        }
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        self.cursor = self.text.len();
        true
    }

    /// Removes `text[..cursor]`; the cursor becomes 0. Both Prompt's "kill to start" and
    /// `Mode::Canonical`'s POSIX KILL ("discard the whole line") are this same operation, since the
    /// cursor is always at the end in `Canonical` -- see this module's and `line_discipline.rs`'s
    /// doc comments.
    fn kill_to_start(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
        true
    }

    /// Removes `text[cursor..]`; the cursor doesn't move. Prompt-only -- POSIX canonical mode has no
    /// kill-to-end character.
    fn kill_to_end(&mut self) -> bool {
        if self.cursor == self.text.len() {
            return false;
        }
        self.text.truncate(self.cursor);
        true
    }

    /// Feeds one token into the buffer, returning what happened, if anything. `mode` gates the
    /// keys that only mean something in `Mode::Prompt` (movement, Ctrl+A/E/K): each returns `None`
    /// directly when `mode` is wrong, rather than falling through to the character-insertion arm
    /// below -- both because they aren't characters (avoids depending on `Token::char()`, which
    /// needs live kernel key-name state `line.rs`'s own host tests don't have) and because it
    /// matches this doc comment's claim that they're simply absent from `Mode::Canonical`'s smaller
    /// key vocabulary, not "recognized then blocked." Ctrl+U (`kill_to_start`) has no such guard,
    /// since it's correct in both modes already. Up/Down aren't matched here at all: they replace
    /// the buffer via history rather than editing it, so `line_discipline.rs` handles them directly.
    pub fn feed(&mut self, token: Token, mode: Mode) -> Option<LineEvent> {
        match token.code {
            KEY_ENTER => Some(LineEvent::Finished(self.take())),
            KEY_BACKSPACE => self.backspace().then_some(LineEvent::Changed),
            KEY_DELETE => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.delete_forward().then_some(LineEvent::Changed)
            }
            KEY_LEFT => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_left().then_some(LineEvent::CursorMoved)
            }
            KEY_RIGHT => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_right().then_some(LineEvent::CursorMoved)
            }
            KEY_HOME => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_home().then_some(LineEvent::CursorMoved)
            }
            KEY_END => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_end().then_some(LineEvent::CursorMoved)
            }
            KEY_A if token.ctrl => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_home().then_some(LineEvent::CursorMoved)
            }
            KEY_E if token.ctrl => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.move_end().then_some(LineEvent::CursorMoved)
            }
            KEY_U if token.ctrl => self.kill_to_start().then_some(LineEvent::Changed),
            KEY_K if token.ctrl => {
                if mode != Mode::Prompt {
                    return None;
                }
                self.kill_to_end().then_some(LineEvent::Changed)
            }
            _ => {
                // Ctrl/Alt held: a shortcut, not text to insert -- e.g. Ctrl+C shouldn't
                // silently type a literal 'c' into the command line.
                if token.ctrl || token.alt {
                    return None;
                }
                let c = token.char()?;
                // Control characters (Tab is the only one `char()` can currently produce) are
                // excluded too -- not for rendering-correctness (`Console::write_char` already
                // handles that safely), but because e.g. a tab-stop jump reads back visually
                // identical to several literal spaces once it's in the buffer, with no way to
                // tell them apart later. Nothing defines an action for Tab yet; a no-op
                // is simplest until it becomes e.g. completion.
                if c.is_control() {
                    return None;
                }
                self.insert(c);
                Some(LineEvent::Changed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(code: u16) -> Token {
        Token {
            code,
            shift: false,
            ctrl: false,
            alt: false,
            caps: false,
        }
    }

    fn ctrl_tok(code: u16) -> Token {
        Token {
            code,
            shift: false,
            ctrl: true,
            alt: false,
            caps: false,
        }
    }

    #[test]
    fn an_unbound_ctrl_combination_is_ignored() {
        // A ctrl-held code with no defined shortcut short-circuits before ever calling
        // `Token::char()` (which needs live kernel key-name state this test doesn't have -- see
        // `feed`'s doc comment), same as a real unbound Ctrl+<key> falling on the floor.
        let mut b = LineBuffer::new();
        b.set("ac");
        b.cursor = 1;
        assert!(b.feed(ctrl_tok(9999), Mode::Prompt).is_none());
        assert_eq!(b.as_str(), "ac");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn backspace_and_delete_forward_are_no_ops_at_the_boundaries() {
        let mut b = LineBuffer::new();
        b.set("ab");
        b.cursor = 0;
        assert!(b.feed(tok(KEY_BACKSPACE), Mode::Prompt).is_none());
        b.cursor = 2;
        assert!(b.feed(tok(KEY_DELETE), Mode::Prompt).is_none());
        assert_eq!(b.as_str(), "ab");
    }

    #[test]
    fn backspace_removes_the_character_before_the_cursor() {
        let mut b = LineBuffer::new();
        b.set("abc");
        b.cursor = 2; // between 'b' and 'c'
        assert!(matches!(
            b.feed(tok(KEY_BACKSPACE), Mode::Prompt),
            Some(LineEvent::Changed)
        ));
        assert_eq!(b.as_str(), "ac");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn delete_forward_removes_the_character_at_the_cursor_without_moving_it() {
        let mut b = LineBuffer::new();
        b.set("abc");
        b.cursor = 1; // between 'a' and 'b'
        assert!(matches!(
            b.feed(tok(KEY_DELETE), Mode::Prompt),
            Some(LineEvent::Changed)
        ));
        assert_eq!(b.as_str(), "ac");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn move_left_right_home_end_report_no_ops_at_the_boundaries() {
        let mut b = LineBuffer::new();
        b.set("ab");
        b.cursor = 0;
        assert!(b.feed(tok(KEY_LEFT), Mode::Prompt).is_none());
        assert!(b.feed(tok(KEY_HOME), Mode::Prompt).is_none());
        assert!(matches!(
            b.feed(tok(KEY_RIGHT), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        ));
        assert_eq!(b.cursor(), 1);
        b.cursor = 2;
        assert!(b.feed(tok(KEY_RIGHT), Mode::Prompt).is_none());
        assert!(b.feed(tok(KEY_END), Mode::Prompt).is_none());
        assert!(matches!(
            b.feed(tok(KEY_LEFT), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        ));
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn ctrl_a_and_ctrl_e_move_home_and_end() {
        let mut b = LineBuffer::new();
        b.set("abc");
        b.cursor = 1;
        assert!(matches!(
            b.feed(ctrl_tok(KEY_A), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        ));
        assert_eq!(b.cursor(), 0);
        assert!(matches!(
            b.feed(ctrl_tok(KEY_E), Mode::Prompt),
            Some(LineEvent::CursorMoved)
        ));
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn ctrl_k_kills_to_end_prompt_only() {
        let mut b = LineBuffer::new();
        b.set("abcd");
        b.cursor = 1;
        assert!(matches!(
            b.feed(ctrl_tok(KEY_K), Mode::Prompt),
            Some(LineEvent::Changed)
        ));
        assert_eq!(b.as_str(), "a");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn ctrl_u_kills_to_start_in_both_modes() {
        for mode in [Mode::Prompt, Mode::Canonical] {
            let mut b = LineBuffer::new();
            b.set("abcd");
            b.cursor = 3;
            assert!(matches!(
                b.feed(ctrl_tok(KEY_U), mode),
                Some(LineEvent::Changed)
            ));
            assert_eq!(b.as_str(), "d");
            assert_eq!(b.cursor(), 0);
        }
    }

    #[test]
    fn movement_and_ctrl_a_e_k_are_unrecognized_in_canonical_mode() {
        let mut b = LineBuffer::new();
        b.set("abc");
        b.cursor = 1;
        for token in [
            tok(KEY_LEFT),
            tok(KEY_RIGHT),
            tok(KEY_HOME),
            tok(KEY_END),
            tok(KEY_DELETE),
            ctrl_tok(KEY_A),
            ctrl_tok(KEY_E),
            ctrl_tok(KEY_K),
        ] {
            assert!(b.feed(token, Mode::Canonical).is_none());
        }
        assert_eq!(b.as_str(), "abc");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn set_replaces_the_text_and_puts_the_cursor_at_the_end() {
        let mut b = LineBuffer::new();
        b.set("ab");
        b.cursor = 0;
        b.set("xyz");
        assert_eq!(b.as_str(), "xyz");
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn take_clears_the_buffer_and_resets_the_cursor() {
        let mut b = LineBuffer::new();
        b.set("abc");
        b.cursor = 1;
        let taken = b.take();
        assert_eq!(taken, "abc");
        assert_eq!(b.as_str(), "");
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn enter_finishes_the_line_and_empties_the_buffer() {
        let mut b = LineBuffer::new();
        b.set("run me");
        match b.feed(tok(KEY_ENTER), Mode::Prompt) {
            Some(LineEvent::Finished(text)) => assert_eq!(text, "run me"),
            _ => panic!("expected Finished"),
        }
        assert!(b.is_empty());
        assert_eq!(b.cursor(), 0);
    }
}
