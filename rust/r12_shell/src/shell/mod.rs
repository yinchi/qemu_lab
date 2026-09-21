//! The shell: everything above the syscalls. `launch` starts programs, `argv` splits a typed line
//! into a command and its arguments, and `handle_keyboard_irq` (for now -- `Stage12.md`'s Step 5
//! moves it out of interrupt context) is the read-eval loop: it feeds key presses to the line being
//! typed, and runs the line when Enter finishes it.
//!
//! The prompt belongs here, not to `keyboard/`: the keyboard side edits and echoes a line, given
//! whatever prefix to draw before it (`console::show_row`'s `prefix`); what the prompt says, and when
//! a fresh one is drawn, is shell policy.

pub mod argv;
pub mod launch;

use crate::console::{BG, FG, show_row};
use crate::fs::blkio::VOL;
use crate::keyboard::events;
use crate::keyboard::line::{INPUT_ROW, LINE, LineEvent};
use crate::platform::globals::{CONSOLE, GPU, KEYBOARD};
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::{static_mut_ref, static_ref};
use argv::{Argv, ParseError};
use launch::{launch, report};

/// The prompt shown before the line buffer -- fixed text with no relation to `LINE`'s own
/// content, so it's structurally impossible for Backspace (which only ever pops `LINE`, see
/// `keyboard/line.rs`) to erase into or through it.
pub const PROMPT: &str = "> ";

/// Handles a keyboard interrupt. Drains every pending event (one IRQ can cover several -- see
/// `Keyboard::poll`), turns each key press into a `Token` (`events::token_for`) and feeds it to
/// `LINE` (`keyboard/line.rs`):
/// - `LineEvent::Changed`: redraw the live line at `INPUT_ROW`.
/// - `LineEvent::Finished`: move off the input row, then run the line (`launch`, which may run a
///   whole program) or report a parse error, resync `INPUT_ROW` to wherever the console cursor
///   actually ended up (a program's output can span any number of rows), and draw a fresh prompt.
///
/// Each event is drawn as it's processed, so several Enters in one batch each launch in turn; only
/// `GPU.flush()` waits for the end of the batch. The UART gets a readable transcript (prompt, each
/// finished line, program output, `report` messages) but no per-keystroke echo.
///
/// SAFETY: one `irq_handler` runs at a time (single core, IRQs masked on entry), and nothing else
/// touches KEYBOARD/CONSOLE/GPU/KEY_STATE/LOCK_STATE/LINE/INPUT_ROW once `kernel_main` enables the
/// keyboard's GIC line, except `read(0)` (`keyboard/stdin.rs`), which only runs inside a program
/// with IRQs masked. `process::run` keeps IRQs masked for a program's whole time at EL0 precisely
/// so this can't be re-entered while `launch` is still on the stack (see its doc comment).
pub fn handle_keyboard_irq() {
    // SAFETY: see this function's doc comment.
    let kb = unsafe { static_mut_ref!(KEYBOARD) };
    kb.ack_interrupt();

    let mut needs_flush = false;

    while let Some(event) = kb.poll() {
        let Some(token) = events::token_for(event) else {
            continue;
        };

        // SAFETY: see this function's doc comment.
        let line = unsafe { static_mut_ref!(LINE) };
        match line.feed(token) {
            Some(LineEvent::Changed) => {
                // SAFETY: see this function's doc comment.
                unsafe {
                    let row = INPUT_ROW;
                    show_row(
                        static_mut_ref!(CONSOLE),
                        row,
                        PROMPT,
                        static_ref!(LINE).as_str(),
                    );
                }
                needs_flush = true;
            }
            Some(LineEvent::Finished(text)) => {
                // SAFETY: see this function's doc comment.
                let console = unsafe { static_mut_ref!(CONSOLE) };

                // The typed line goes to the UART here, once finished, following the prompt
                // already written there -- a readable transcript, without echoing every
                // keystroke (see `show_row`).
                uart_write(text.as_bytes());
                uart_write(b"\n");

                // Move off the just-finished input row *before* anything below writes a
                // single byte -- otherwise a launched program's own output (or an error
                // message) starts writing right where the just-typed line's cursor was left
                // sitting, running straight into its tail instead of starting its own row.
                console.write_char('\n', FG, BG);

                match Argv::parse(&text) {
                    Ok(argv) => {
                        // SAFETY: see this function's doc comment.
                        let vol = unsafe { static_ref!(VOL) };
                        launch(vol, &argv, console);
                    }
                    // A blank line (just Enter with nothing typed) isn't an error --
                    // nothing to log, same as any real shell.
                    Err(ParseError::Empty) => {}
                    Err(ParseError::Malformed) => {
                        report(console, &alloc::format!("Malformed input: {text:?}"));
                    }
                }
                // Resync INPUT_ROW to wherever the console's own cursor actually ended up --
                // unchanged if nothing ran (still sitting at the end of the just-finished
                // prompt+line, from the last Changed redraw), or wherever `launch`'s program
                // output (fd::write, via Console::write_char -- itself now scroll-safe, see
                // its doc comment) left it, however many rows that spanned. A trailing
                // newline only if not already at column 0, so a program whose own last write
                // already ended in '\n' doesn't get a spurious blank line.
                if console.cursor().1 != 0 {
                    console.write_char('\n', FG, BG);
                }
                let row = console.cursor().0;
                // SAFETY: see this function's doc comment.
                unsafe {
                    INPUT_ROW = row;
                }
                show_row(console, row, PROMPT, "");
                uart_ensure_newline();
                uart_write(PROMPT.as_bytes());
                needs_flush = true;
            }
            None => {}
        }
    }

    if needs_flush {
        // SAFETY: see this function's doc comment.
        unsafe { static_mut_ref!(GPU) }.flush();
    }
}
