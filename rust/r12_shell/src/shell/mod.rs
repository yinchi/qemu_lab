//! The shell: everything above the syscalls. `launch` starts programs, `argv` splits a typed line
//! into a command and its arguments, and `handle_keyboard_irq` (for now -- `Stage12.md`'s Step 5
//! moves it out of interrupt context) is the read-eval loop: it hands key presses to the line
//! discipline, and runs the line when Enter finishes it.
//!
//! The prompt belongs here, not to `keyboard/`: the line discipline (`keyboard/line_discipline.rs`) edits
//! and echoes a line, given whatever prefix to draw before it; what the prompt says, and when a
//! fresh one is drawn, is shell policy.

pub mod argv;
pub mod launch;

use crate::console::Console;
use crate::fs::blkio::VOL;
use crate::keyboard::events;
use crate::keyboard::line_discipline::{LINE_DISCIPLINE, LineDiscipline, LineOutcome, Mode};
use crate::platform::globals::{CONSOLE, GPU, KEYBOARD};
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::{static_mut_ref, static_ref};
use argv::{Argv, ParseError};
use launch::{launch, report};

/// The prompt shown before the line being typed -- fixed text with no relation to the line's own
/// content, so it's structurally impossible for Backspace (which only ever pops the line buffer, see
/// `keyboard/line.rs`) to erase into or through it.
pub const PROMPT: &str = "> ";

/// Starts a fresh prompt: a new input line on the console (on a fresh row if whatever ran left the
/// cursor mid-line), the prompt drawn on it, and the UART's transcript given the prompt too.
/// Called once at boot and after every line the shell finishes with. Does not flush the display.
pub fn start_prompt(discipline: &mut LineDiscipline, console: &mut Console) {
    discipline.begin(console, PROMPT, Mode::Prompt);
    discipline.redraw(console);
    uart_ensure_newline();
    uart_write(PROMPT.as_bytes());
}

/// Handles a keyboard interrupt. Drains every pending event (one IRQ can cover several -- see
/// `Keyboard::poll`), turns each key press into a `Token` (`events::token_for`) and hands it to the
/// line discipline (`keyboard/line_discipline.rs`), which edits, echoes and finishes the line:
/// - `LineOutcome::Edited`: nothing more to do here but flush.
/// - `LineOutcome::Finished`: run the line (`launch`, which may run a whole program) or report a
///   parse error, then start a fresh prompt wherever the console cursor actually ended up (a
///   program's output can span any number of rows).
///
/// Each event is handled as it's processed, so several Enters in one batch each launch in turn; only
/// `GPU.flush()` waits for the end of the batch. The UART gets a readable transcript (prompt, each
/// finished line, program output, `report` messages) but no per-keystroke echo.
///
/// SAFETY: one `irq_handler` runs at a time (single core, IRQs masked on entry), and nothing else
/// touches KEYBOARD/CONSOLE/GPU/KEY_STATE/LOCK_STATE/LINE_DISCIPLINE once `kernel_main` enables the
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
        let (discipline, console) =
            unsafe { (static_mut_ref!(LINE_DISCIPLINE), static_mut_ref!(CONSOLE)) };
        match discipline.handle(token, console) {
            LineOutcome::Ignored | LineOutcome::EndOfFile => {}
            LineOutcome::Edited => needs_flush = true,
            LineOutcome::Finished(text) => {
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
                start_prompt(discipline, console);
                needs_flush = true;
            }
        }
    }

    if needs_flush {
        // SAFETY: see this function's doc comment.
        unsafe { static_mut_ref!(GPU) }.flush();
    }
}
