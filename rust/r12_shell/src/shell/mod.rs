//! The shell: everything above the syscalls. `launch` starts programs, `argv` splits a typed line
//! into a command and its arguments, and `run` is the read-eval loop `kernel_main` ends in and never
//! leaves (the role `init` plays): it takes key presses from the token queue, hands them to the line
//! discipline, and runs the line when Enter finishes it. It is ordinary code running outside any
//! interrupt -- the keyboard interrupt only feeds the queue (`keyboard/queue.rs`) -- so a program it
//! launches runs with interrupts enabled.
//!
//! The prompt belongs here, not to `keyboard/`: the line discipline (`keyboard/line_discipline.rs`) edits
//! and echoes a line, given whatever prefix to draw before it; what the prompt says, and when a
//! fresh one is drawn, is shell policy.

pub mod builtins;
pub mod launch;
pub mod lexer;
pub mod syntax;

use crate::console::Console;
use crate::fs::blkio::VOL;
use crate::keyboard::line_discipline::{LINE_DISCIPLINE, LineDiscipline, LineOutcome, Mode};
use crate::keyboard::queue;
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::{static_mut_ref, static_ref};
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

/// Runs one typed line: parses it (`syntax.rs`) and executes it -- a builtin (`builtins.rs`) or a program
/// (`launch.rs`) -- reporting whatever went wrong as one line of text. A blank line or a comment does
/// nothing, quietly, as in any shell. Pipes and redirections parse but are not run yet, and say so.
pub fn run_line(line: &str, console: &mut Console) {
    let pipeline = match syntax::parse(line) {
        Ok(Some(pipeline)) => pipeline,
        Ok(None) => return,
        Err(error) => {
            report(console, &alloc::format!("syntax error: {error}"));
            return;
        }
    };
    if pipeline.len() > 1 {
        report(console, "pipes are not supported yet");
        return;
    }
    let segment = &pipeline[0];
    if !segment.redirs.is_empty() {
        report(console, "redirection is not supported yet");
        return;
    }
    let argv: alloc::vec::Vec<&str> = segment
        .argv
        .iter()
        .map(alloc::string::String::as_str)
        .collect();
    if builtins::is_builtin(argv[0]) {
        if let Err(message) = builtins::run(argv[0], &argv[1..]) {
            report(console, &message);
        }
    } else {
        // SAFETY: as `run`'s doc comment says of the statics it uses.
        let vol = unsafe { static_ref!(VOL) };
        launch(vol, &argv, console);
    }
}

/// The read-eval loop: takes each key press from the token queue, hands it to the line discipline
/// (`keyboard/line_discipline.rs`), which edits, echoes and finishes the line, and acts on the outcome:
/// - `LineOutcome::Edited`: nothing more to do but flush the display once the queue is empty.
/// - `LineOutcome::Finished`: run the line (`launch`, which may run a whole program) or report a parse
///   error, then start a fresh prompt wherever the console cursor actually ended up (a program's
///   output can span any number of rows).
///
/// Keys pressed while a program runs, or while the shell is busy, wait in the queue and are handled in
/// order afterwards -- several Enters queued up each launch in turn. The display is flushed when the
/// queue runs dry, so a burst of keys costs one flush. When there is nothing to do it sleeps until an
/// interrupt. The UART gets a readable transcript (prompt, each finished line, program output,
/// `report` messages) but no per-keystroke echo.
///
/// SAFETY of the statics used: this loop and, from inside `launch`, a program's `read(0)` are the
/// only users of the console, the display and the line discipline, and never at the same time; the
/// interrupt handler touches none of them (it only feeds the queue).
pub fn run() -> ! {
    loop {
        let mut needs_flush = false;

        while let Some(token) = queue::pop() {
            // SAFETY: see this function's doc comment.
            let (discipline, console) =
                unsafe { (static_mut_ref!(LINE_DISCIPLINE), static_mut_ref!(CONSOLE)) };
            match discipline.handle(token, console) {
                LineOutcome::Ignored | LineOutcome::EndOfFile => {}
                LineOutcome::Edited => needs_flush = true,
                LineOutcome::Finished(text) => {
                    run_line(&text, console);
                    start_prompt(discipline, console);
                    needs_flush = true;
                }
            }
        }

        if needs_flush {
            // SAFETY: see this function's doc comment.
            unsafe { static_mut_ref!(GPU) }.flush();
        }
        queue::wait_for_token();
    }
}
