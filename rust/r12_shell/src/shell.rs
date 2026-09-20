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

/// Handles a keyboard interrupt: drains every pending event (one IRQ can cover more than one --
/// see `drivers::virtio::input::Keyboard::poll`'s doc comment), turns each genuine press into a
/// token (`events::token_for`, which also updates the held-key set and the lock-key toggles) and
/// feeds it into `LINE` (see `keyboard/line.rs` for what that does with it):
/// - A plain edit (`LineEvent::Changed`) redraws `INPUT_ROW` in place with the live line.
/// - A finished line (`LineEvent::Finished`) always moves off the input row first (an
///   unconditional `'\n'`), then either runs `launch` -- which may load and run a whole program,
///   producing output of its own via `syscall/fd.rs` -- or reports a parse error via `report`, then
///   resyncs `INPUT_ROW` to wherever the console's cursor *actually* ended up (not simply "one
///   row down": a launched program's output can span an arbitrary number of rows) before
///   drawing a fresh prompt there.
///
/// The UART is kept as a readable transcript: the prompt, each finished line, whatever a launched
/// program prints (mirrored by `syscall/fd.rs`), and `report`'s messages -- but no running echo of
/// every keystroke. Auto-repeat (`value == 2`) intentionally reaches none of this, the same as it's
/// already a no-op for `KeyState`/`LockState` (see `events::token_for`).
///
/// Drawing happens immediately per event, not deferred to a single redraw after the drain loop:
/// a batch containing more than one Enter needs each one to actually advance/scroll/launch in
/// turn, not collapse into one. `GPU.flush()` alone is still deferred to the end of the batch --
/// it's a presentation step, not something drawing operations need in between to stay correct.
///
/// SAFETY: at most one `irq_handler` invocation runs at a time (single core, and taking an IRQ
/// exception masks further IRQs for its duration), and nothing outside `irq_handler` touches
/// KEYBOARD/CONSOLE/GPU/KEY_STATE/LOCK_STATE/LINE/INPUT_ROW from the point `kernel_main` enables
/// KEYBOARD_SPI's GIC line onward, except `keyboard/stdin.rs`'s `read(0)` -- which only ever runs
/// during a program, with every IRQ masked -- so these `static mut` accesses can't race anything.
/// `launch`'s `process::run_program` is what actually upholds this while a program runs: it
/// masks every DAIF bit for the program's entire time at EL0 (see `exec/process.rs`'s doc comment),
/// specifically so a keyboard IRQ can never land mid-program and re-enter this function while an
/// outer call is still on the stack, blocked inside `run_program` -- that would otherwise remap
/// the fixed user window a program is currently executing out of, and stomp the single-slot
/// `KERNEL_CTX` checkpoint (`arch/context.s`) its own `enter_el0` just wrote.
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
