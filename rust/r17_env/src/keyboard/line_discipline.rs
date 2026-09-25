//! The line discipline: what turns a stream of `Token`s into an edited line on the console. It owns the
//! line being typed (`LineBuffer`), which console row it is on, how it is echoed and redrawn, and what
//! finishing it looks like -- the newline on the console and the transcript on the UART. The shell's prompt
//! and a running program's `read(0)` both call it, so a line is typed, edited, drawn and finished
//! identically whichever of them is waiting for it. (Before Step 4 each had its own copy of this.)
//!
//! A line longer than a row wraps onto the rows below it, as on any terminal (Step 4b; the layout comes
//! from `console/input_layout.rs`), and may not outgrow the screen: past `rows - 1` rows, further
//! characters are ignored.
//!
//! What a key does depends on the `Mode` (`line.rs`): `Mode::Prompt` (the shell's prompt) edits at a
//! movable cursor and recalls history (`history.rs`); `Mode::Canonical` (a program's `read(0)`) is a
//! real tty's cooked mode -- Backspace, Ctrl+U, Ctrl+D, no cursor movement, no history. The full key
//! table is in `docs/console.md`. The cursor is drawn as an inverted cell, and un-inverted when a line
//! finishes.
//!
//! What the callers keep to themselves: where tokens come from, what a finished line means (the shell runs
//! it, `read(0)` hands it to the program), and the prompt. The rules that keep this module useful for
//! the stages after it:
//!
//! - It knows nothing about where tokens come from -- never the keyboard device, `wfi`, or interrupt
//!   state. Its callers pop them from the token queue (`queue.rs`).
//! - It is never called from interrupt context. The IRQ handler only enqueues tokens; this module runs
//!   only in the shell's loop or inside a syscall, one at a time, which is why it needs no lock.
//! - It never sees signal keys (Ctrl+Z, Ctrl+C): the queue's producer recognizes those before a token is
//!   queued (Stages 21-23). Ctrl+D is different -- end-of-file is canonical-mode policy, so it is handled
//!   here.
//! - There is one instance, for the one console; what varies between uses is the prefix and the `Mode`
//!   passed to `begin`. Who is *reading* (the shell, a program, later a raw-mode editor that bypasses
//!   this entirely) is a routing decision made above it.
//! - It is a keyboard-side module: it draws on the `console`, and the console never calls back.

use alloc::string::String;

use super::history::History;
use super::line::{LineBuffer, LineEvent};
pub use super::line::Mode;
use super::tokens::{KEY_D, KEY_DOWN, KEY_UP, Token};
use crate::console::input_layout::{cursor_position, fits_on_screen, rows_needed};
use crate::console::{BG, Console, FG};
use crate::platform::uart::uart_write;

/// What handling one token did.
pub enum LineOutcome {
    /// Nothing that shows: a modifier, a shortcut, an unbound key, Backspace on an empty line.
    Ignored,
    /// The line changed (text or cursor) and was redrawn; the display needs a flush.
    Edited,
    /// Enter finished the line -- here is its text; the console has moved to the next row and the UART has
    /// the line. The display needs a flush.
    Finished(String),
    /// Ctrl+D on an empty line in `Mode::Canonical`. The line itself isn't redrawn (there was
    /// nothing to show differently), only the visible cursor cell is un-inverted if it was showing
    /// -- the caller flushes if it wants that visible right away; `read_line` doesn't, since the
    /// program reading almost always exits or blocks right after, and the shell's next prompt
    /// flushes on its way up regardless.
    EndOfFile,
    /// Ctrl+D on a *non-empty* line in `Mode::Canonical`: here is what had been typed so far, with
    /// no trailing newline -- POSIX's actual rule (a further Ctrl+D on the now-empty line is
    /// `EndOfFile`). The line's text isn't redrawn (it stays on screen exactly as typed), but the
    /// visible cursor cell (always at the end, in `Mode::Canonical`) is un-inverted, the same as
    /// `Finished` -- unlike `EndOfFile`, the reading program keeps running afterward and may not
    /// touch the display again for a while, so the caller should flush this one.
    Partial(String),
}

/// The line being typed, on the console.
pub struct LineDiscipline {
    /// The text typed so far.
    buffer: LineBuffer,
    /// The console row the live prefix + line starts on. A finished line is never redrawn or erased --
    /// it is already showing correctly from the edits leading up to Enter -- so finishing only ever
    /// needs the next `begin` to pick a new row.
    row: usize,
    /// How many rows the prefix + line occupied when last drawn (at least one): what a redraw must
    /// clear, since a line that just got shorter leaves its old last rows behind.
    height: usize,
    /// What is drawn before the line: the shell's prompt, or nothing for a program.
    prefix: String,
    mode: Mode,
    /// The shell prompt's command history (`Mode::Prompt`'s Up/Down; untouched in `Mode::Canonical`,
    /// since a program's `read(0)` never records or recalls anything).
    history: History,
}

/// The line discipline of the one console. Written once by `kernel_main`, before the keyboard's
/// interrupt is enabled.
/// SAFETY (every access): single core; callers are the shell's handler and `read(0)`, which never run
/// at the same time (see this module's doc comment).
pub static mut LINE_DISCIPLINE: Option<LineDiscipline> = None;

impl LineDiscipline {
    pub const fn new() -> Self {
        Self {
            buffer: LineBuffer::new(),
            row: 0,
            height: 1,
            prefix: String::new(),
            mode: Mode::Prompt,
            history: History::new(),
        }
    }

    /// Starts a new line with `prefix` in front of it: on a fresh row if whatever ran before left the
    /// cursor mid-line, else on the cursor's own row. Draws nothing (see `redraw`); the buffer is empty
    /// -- a finished line already emptied it. Also ends any in-progress history browsing, so a stale
    /// recall position from a previous line never leaks into this one.
    pub fn begin(&mut self, console: &mut Console, prefix: &str, mode: Mode) {
        if console.cursor().1 != 0 {
            console.write_char('\n', FG, BG);
        }
        self.row = console.cursor().0;
        self.height = 1;
        self.prefix.clear();
        self.prefix.push_str(prefix);
        self.mode = mode;
        self.history.reset_recall();
    }

    /// Draws the prefix and the line so far, from the line's first row down as far as it needs: the rows
    /// it used to occupy are cleared first, then the text is written the way any output is, so it wraps
    /// at the edge of the screen and scrolls it if it runs past the bottom -- after which the first row
    /// is recomputed from where the cursor ended, because scrolling moved the line up. Finishes by
    /// placing the visible cursor at its logical position (`self.buffer.cursor()`), which is not
    /// necessarily where `write_char` just left the console's own cursor -- the user may have moved left.
    pub fn redraw(&mut self, console: &mut Console) {
        let height = rows_needed(&self.prefix, self.buffer.as_str(), console.cols);
        let last_row = (self.row + self.height.max(height)).min(console.rows);
        for row in self.row..last_row {
            console.clear_row(row, BG);
        }
        console.move_cursor(self.row, 0);
        for c in self.prefix.chars().chain(self.buffer.as_str().chars()) {
            console.write_char(c, FG, BG);
        }
        self.row = console.cursor().0 + 1 - height;
        self.height = height;
        self.draw_cursor(console);
    }

    /// Draws the cell at `cursor` (a byte offset, on a char boundary) within `text`, `inverted` or
    /// in ordinary colors: whatever character is actually there, or a space past the end of `text`.
    /// Wide glyphs need no special handling here: `cursor_position` always returns a glyph's *left*
    /// anchor cell (never the right half of a wide one), and `put_char_at` already draws
    /// width-aware, the same way ordinary typing does (see `console/mod.rs`). Takes `text`
    /// explicitly rather than always reading `self.buffer`, since un-inverting a line's last cursor
    /// cell when it finishes (`handle`'s `Finished` arm) needs the text as it was *before* `feed`
    /// cleared the buffer -- `self.buffer.as_str()` is already empty by then.
    fn draw_cell_at(&self, console: &mut Console, text: &str, cursor: usize, inverted: bool) {
        let (rel_row, col) = cursor_position(&self.prefix, text, cursor, console.cols);
        let c = text[cursor..].chars().next().unwrap_or(' ');
        let (fg, bg) = if inverted { (BG, FG) } else { (FG, BG) };
        console.put_char_at(self.row + rel_row, col, c, fg, bg);
    }

    /// Draws the visible cursor (inverse video: `fg`/`bg` swapped) at its current logical position.
    fn draw_cursor(&self, console: &mut Console) {
        self.draw_cell_at(console, self.buffer.as_str(), self.buffer.cursor(), true);
    }

    /// A pure cursor move: un-inverts the old cursor cell (drawing it in ordinary colors), then
    /// draws the new one -- two `put_char_at` calls, regardless of how far apart they are (e.g. End
    /// pressed from row 0 landing on row 3 costs the same as a same-row move; see
    /// `console/input_layout.rs`'s Home/End note). `old_cursor` is a byte offset into the *current*
    /// text, valid because `LineEvent::CursorMoved` only ever fires when the text itself didn't change.
    fn redraw_cursor(&self, console: &mut Console, old_cursor: usize) {
        self.draw_cell_at(console, self.buffer.as_str(), old_cursor, false);
        self.draw_cursor(console);
    }

    /// Handles one token. The caller flushes the display afterwards if the outcome says so.
    pub fn handle(&mut self, token: Token, console: &mut Console) -> LineOutcome {
        if self.mode == Mode::Canonical && token.ctrl && token.code == KEY_D {
            // Un-invert wherever the cursor was last drawn -- same reason as the `Finished` arm
            // below: nothing else redraws this row before the caller moves on (`read`'s next call,
            // if any, starts a fresh one via `begin`), so a stale inverted cell would otherwise be
            // left behind permanently.
            let cursor = self.buffer.cursor();
            return if self.buffer.is_empty() {
                self.draw_cell_at(console, "", cursor, false);
                LineOutcome::EndOfFile
            } else {
                let text = self.buffer.take();
                self.draw_cell_at(console, &text, cursor, false);
                LineOutcome::Partial(text)
            };
        }

        if self.mode == Mode::Prompt
            && !token.ctrl
            && !token.alt
            && (token.code == KEY_UP || token.code == KEY_DOWN)
        {
            let recalled = if token.code == KEY_UP {
                self.history.recall_prev(self.buffer.as_str())
            } else {
                self.history.recall_next()
            };
            return match recalled {
                None => LineOutcome::Ignored,
                Some(text) => {
                    let text = String::from(text);
                    self.buffer.set(&text);
                    self.redraw(console);
                    LineOutcome::Edited
                }
            };
        }

        let old_cursor = self.buffer.cursor();
        match self.buffer.feed(token, self.mode) {
            None => LineOutcome::Ignored,
            Some(LineEvent::CursorMoved) => {
                self.redraw_cursor(console, old_cursor);
                LineOutcome::Edited
            }
            Some(LineEvent::Changed) => {
                // A character that would make the line outgrow the screen is dropped -- the only
                // way `Changed` can overflow, since every other edit it covers (Backspace, Delete,
                // Ctrl+U/K) only ever shortens the line.
                if !fits_on_screen(
                    &self.prefix,
                    self.buffer.as_str(),
                    console.cols,
                    console.rows,
                ) {
                    self.buffer.backspace();
                    return LineOutcome::Ignored;
                }
                self.redraw(console);
                LineOutcome::Edited
            }
            Some(LineEvent::Finished(text)) => {
                // Un-invert wherever the cursor was last drawn (usually the end, but Enter can be
                // pressed with the cursor anywhere) -- `redraw` is never called again for a
                // finished line (see this struct's `row` doc comment), so nothing else would.
                self.draw_cell_at(console, &text, old_cursor, false);
                // Only the shell's prompt has a history: what a program reads with `read(0)` is
                // that program's input, not a command to recall.
                if self.mode == Mode::Prompt {
                    self.history.record(&text);
                    self.history.reset_recall();
                }
                // The typed line goes to the UART here, once finished -- a readable transcript
                // without echoing every keystroke -- and the console moves off the input row
                // *before* anything else writes a byte, so a launched program's output (or an error
                // message) starts its own row instead of running into the tail of the typed line.
                uart_write(text.as_bytes());
                uart_write(b"\n");
                console.write_char('\n', FG, BG);
                LineOutcome::Finished(text)
            }
        }
    }
}
