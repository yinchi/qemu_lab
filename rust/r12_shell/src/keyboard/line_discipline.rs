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
//! What the callers keep to themselves: where tokens come from, what a finished line means (the shell runs
//! it, `read(0)` hands it to the program), and the prompt. The rules that keep this module useful for
//! the stages after it:
//!
//! - It knows nothing about where tokens come from -- never the keyboard device, `wfe`, or interrupt
//!   state. Today its callers drain the device themselves; Step 5 has them pop a queue instead, and
//!   nothing here changes.
//! - It is never called from interrupt context. Once Step 5 lets IRQs through while a program runs, the
//!   IRQ handler only enqueues tokens; this module runs only in the shell's loop or inside a syscall,
//!   one at a time, which is why it needs no lock.
//! - It never sees signal keys (Ctrl+Z, Ctrl+C): the queue's producer recognizes those before a token is
//!   queued (Stages 20-22). Ctrl+D is different -- end-of-file is canonical-mode policy, so it is handled
//!   here.
//! - There is one instance, for the one console; what varies between uses is the prefix and the `Mode`
//!   passed to `begin`. Who is *reading* (the shell, a program, later a raw-mode editor that bypasses
//!   this entirely) is a routing decision made above it.
//! - It is a keyboard-side module: it draws on the `console`, and the console never calls back.

use alloc::string::String;

use super::line::{LineBuffer, LineEvent};
use super::tokens::{KEY_D, Token};
use crate::console::input_layout::{fits_on_screen, rows_needed};
use crate::console::{BG, Console, FG};
use crate::platform::uart::uart_write;

/// How a line is being read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The shell's prompt: Ctrl+D does nothing (the shell is init and never exits on end-of-file).
    Prompt,
    /// A program's `read(0)`, a tty's canonical mode: Ctrl+D on an empty line is end-of-file.
    Canonical,
}

/// What handling one token did.
pub enum LineOutcome {
    /// Nothing that shows: a modifier, a shortcut, an unbound key, Backspace on an empty line.
    Ignored,
    /// The line changed and its row was redrawn; the display needs a flush.
    Edited,
    /// Enter finished the line -- here is its text; the console has moved to the next row and the UART has
    /// the line. The display needs a flush.
    Finished(String),
    /// Ctrl+D on an empty line in `Mode::Canonical`.
    EndOfFile,
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
    prefix: &'static str,
    mode: Mode,
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
            prefix: "",
            mode: Mode::Prompt,
        }
    }

    /// Starts a new line with `prefix` in front of it: on a fresh row if whatever ran before left the
    /// cursor mid-line, else on the cursor's own row. Draws nothing (see `redraw`); the buffer is empty
    /// -- a finished line already emptied it.
    pub fn begin(&mut self, console: &mut Console, prefix: &'static str, mode: Mode) {
        if console.cursor().1 != 0 {
            console.write_char('\n', FG, BG);
        }
        self.row = console.cursor().0;
        self.height = 1;
        self.prefix = prefix;
        self.mode = mode;
    }

    /// Draws the prefix and the line so far, from the line's first row down as far as it needs: the rows
    /// it used to occupy are cleared first, then the text is written the way any output is, so it wraps
    /// at the edge of the screen and scrolls it if it runs past the bottom -- after which the first row
    /// is recomputed from where the cursor ended, because scrolling moved the line up.
    pub fn redraw(&mut self, console: &mut Console) {
        let height = rows_needed(self.prefix, self.buffer.as_str(), console.cols);
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
    }

    /// Handles one token. The caller flushes the display afterwards if the outcome says so.
    pub fn handle(&mut self, token: Token, console: &mut Console) -> LineOutcome {
        if self.mode == Mode::Canonical
            && token.ctrl
            && token.code == KEY_D
            && self.buffer.is_empty()
        {
            return LineOutcome::EndOfFile;
        }
        match self.buffer.feed(token) {
            None => LineOutcome::Ignored,
            Some(LineEvent::Changed) => {
                // A character that would make the line outgrow the screen is dropped (Backspace, the
                // other way a line changes, only ever shortens it).
                if !fits_on_screen(
                    self.prefix,
                    self.buffer.as_str(),
                    console.cols,
                    console.rows,
                ) {
                    self.buffer.pop();
                    return LineOutcome::Ignored;
                }
                self.redraw(console);
                LineOutcome::Edited
            }
            Some(LineEvent::Finished(text)) => {
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
