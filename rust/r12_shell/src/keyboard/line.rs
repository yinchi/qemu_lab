//! Stage 10's bare line-reading loop's buffer: append-only from the keyboard side, with
//! Backspace popping the last character -- Stage 4's "echo line" model, not Stage 5's
//! cursor-aware editing (see `ROADMAP.md`). This is deliberately the *only* place Backspace
//! ever gets handled -- absorbed here, before Enter is ever reached, not forwarded as a raw
//! control byte to anything else. Two callers feed it: the shell's own prompt
//! (`shell::handle_keyboard_irq`) and a running program's `read(0)` (`stdin.rs`), which is how a
//! finished line reaches a program's stdin with Backspace already applied.
//!
//! Deliberately a separate module from `tokens.rs`, same reasoning as that module's own
//! separation from `keymap.rs`: this is one particular interpretation of a `Token` stream (an
//! editable line, stopping at Enter), not the only one a future raw-mode consumer might want.

use alloc::string::String;

use super::tokens::{KEY_BACKSPACE, KEY_ENTER, Token};

/// The line being typed, shared by the shell's prompt and a running program's `read(0)`
/// (`stdin.rs`). Lives across IRQs the same way `KEY_STATE`/`LOCK_STATE` do, for the same reason:
/// one keypress is one `handle_keyboard_irq` call, so the line has to persist somewhere between
/// them.
pub static mut LINE: Option<LineBuffer> = None;

/// Which row the live prompt+line is currently on. A finished line is never redrawn or erased
/// -- it's already showing correctly at this row from the live edits leading up to Enter -- so
/// finishing a line only ever needs to *advance* this to a new row (scrolling first if already
/// on the last one), not touch anything already on screen. Not an `Option` like `LINE`/
/// `KEY_STATE`: `0` is already the correct starting value, the same one `kernel_main`'s first
/// `show_row` call uses, so there's no real "uninitialized" state to model.
pub static mut INPUT_ROW: usize = 0;

/// What feeding one `Token` into the buffer did, if anything -- `None` for a token this buffer
/// doesn't act on (a Ctrl/Alt-held shortcut, a control character, or a key with no character at
/// all).
pub enum LineEvent {
    /// The buffer's visible content changed (an append or a pop) -- redraw it.
    Changed,
    /// Enter finished the line: here's what was typed; the buffer is now empty again.
    Finished(String),
}

pub struct LineBuffer {
    text: String,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            text: String::new(),
        }
    }

    /// The buffer's current content, for display.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Feeds one token into the buffer, returning what happened, if anything.
    pub fn feed(&mut self, token: Token) -> Option<LineEvent> {
        match token.code {
            KEY_ENTER => Some(LineEvent::Finished(core::mem::take(&mut self.text))),
            KEY_BACKSPACE => self.text.pop().map(|_| LineEvent::Changed),
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
                self.text.push(c);
                Some(LineEvent::Changed)
            }
        }
    }
}
