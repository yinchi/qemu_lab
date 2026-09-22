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

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::errno::errmsg;

use crate::console::{BG, Console, FG};
use crate::exec::frame_stack::{FrameStack, StdioBinding};
use crate::exec::shell_state;
use crate::fs::blkio::VOL;
use crate::fs::files;
use crate::keyboard::line_discipline::{LINE_DISCIPLINE, LineDiscipline, LineOutcome, Mode};
use crate::keyboard::queue;
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::{static_mut_ref, static_ref};
use launch::launch;
use syntax::{Redirection, Segment};

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
/// (`launch.rs`) -- reporting whatever went wrong via `shell_err`. A blank line or a comment does
/// nothing, quietly, as in any shell. Pipes parse but are not run yet, and say so.
pub fn run_line(line: &str) {
    run_line_inner(line, 0);
}

/// `run_line`'s real body, with the script-nesting depth threaded through: `0` at the prompt, and
/// `depth + 1` for every line a script (`run_script_content`) feeds back through here. Kept separate
/// from `run_line` so the prompt -- the only caller that doesn't already have a `depth` -- has a
/// plain, depth-free entry point.
fn run_line_inner(line: &str, depth: usize) {
    let pipeline = match syntax::parse(line) {
        Ok(Some(pipeline)) => pipeline,
        Ok(None) => return,
        Err(error) => {
            shell_err(&format!("syntax error: {error}"));
            return;
        }
    };
    if pipeline.len() > 1 {
        shell_err("pipes are not supported yet");
        return;
    }
    run_segment(&pipeline[0], depth);
}

/// Runs one segment: opens its redirections in order -- left to right, each already in effect for
/// the ones after it, so `2>&1 > f` and `> f 2>&1` differ -- then the command itself, all under one
/// `with_stdio` scope. That scope is why a builtin's own state change (`cd`) still sticks under a
/// redirect, why a failed redirect leaves exactly the earlier ones of the same line in effect, and
/// why the error message for that failure (and any launch error) is itself subject to whichever
/// redirects already succeeded -- `cmd 2> e < missing` reports the missing-file error into `e`, not
/// the console. Shell-opened handles are closed once the scope ends, committing anything written.
fn run_segment(segment: &Segment, depth: usize) {
    let mut opened = Vec::new();
    shell_state::frames().with_stdio([None, None, None], |frames| {
        for redir in &segment.redirs {
            if !apply_redirect(frames, redir, &mut opened) {
                return; // shell_err already reported; the command does not run
            }
        }
        run_command(&segment.argv, depth);
    });
    for handle in opened {
        files::close(handle);
    }
}

/// Opens and binds one redirection on `frames`' top frame, recording any handle it opened in
/// `opened` so `run_segment` can close it afterward. Returns whether it succeeded; on failure it has
/// already reported the error via `shell_err`.
fn apply_redirect(frames: &mut FrameStack, redir: &Redirection, opened: &mut Vec<usize>) -> bool {
    match redir {
        Redirection::In(path) => match open_redirect_target(path, false, false) {
            Ok(handle) => {
                opened.push(handle);
                frames.top_mut().stdio[0] = StdioBinding::File(handle);
                true
            }
            Err(msg) => {
                shell_err(&msg);
                false
            }
        },
        Redirection::Out { fd, path, append } => match open_redirect_target(path, true, *append) {
            Ok(handle) => {
                opened.push(handle);
                frames.top_mut().stdio[*fd as usize] = StdioBinding::File(handle);
                true
            }
            Err(msg) => {
                shell_err(&msg);
                false
            }
        },
        Redirection::Dup { fd, target } => {
            let binding = frames.top().stdio[*target as usize];
            frames.top_mut().stdio[*fd as usize] = binding;
            true
        }
    }
}

/// Resolves `path` against the working directory and opens it for a redirection, in bash's wording
/// on failure. Marks the handle shell-owned (`files::mark_shell_owned`) so it survives whatever
/// program runs under this redirect exiting, for `run_segment` to close once the whole segment does.
fn open_redirect_target(path: &str, write: bool, append: bool) -> Result<usize, String> {
    let abspath = shell_state::absolute(path).map_err(|e| format!("{path}: {}", errmsg(e)))?;
    let handle =
        files::open(&abspath, write, append).map_err(|e| format!("{path}: {}", errmsg(e)))?;
    files::mark_shell_owned(handle);
    Ok(handle)
}

/// Runs a segment's command -- once its redirections (if any) are already bound -- as a builtin or a
/// program. Empty `argv` (a stage of only redirections, `> f`) runs nothing: POSIX still creates the
/// file.
fn run_command(argv: &[String], depth: usize) {
    let Some(name) = argv.first() else {
        return;
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    if builtins::is_builtin(name) {
        if let Err(message) = builtins::run(name, &argv[1..], depth) {
            shell_err(&message);
        }
    } else {
        // SAFETY: as `run`'s doc comment says of the statics it uses.
        let vol = unsafe { static_ref!(VOL) };
        launch(vol, &argv, depth);
    }
}

/// How many scripts may be nested (a script's own line launching another script, and so on) before
/// `run_script_content` refuses to go further -- protects the 1 MiB kernel stack from a script that
/// (directly or indirectly) sources or runs itself. Deliberately not `FrameStack::depth`: that only
/// grows for *scoped* nesting (`./script`, `sh script`), and `source`/`.` -- unscoped by design, so a
/// sourced script's `cd` sticks -- never pushes a frame at all, so it would miss exactly the
/// recursion (`source` sourcing itself) most likely to happen.
const MAX_SCRIPT_DEPTH: usize = 16;

/// Runs `content` (a script's lines) as if each were typed at the prompt: blank lines and `#`
/// comments already do nothing and a failing line reports and the script continues, both already
/// `run_line_inner`'s ordinary behavior for a bad or failing line, so nothing special is needed here
/// beyond feeding it every line. `scoped` pushes a frame first (`./script`, `sh script`: what a real
/// child shell process would isolate) so the script's own `cd`s and redirects don't leak past it;
/// unscoped (`source`/`.`) runs against the current frame, so they do. The caller has already found
/// and read the file (finding it works differently for each caller: `launch.rs`'s `./file` fallback
/// already has the bytes it peeked at for the ELF-magic check; `source`/`sh`, in `builtins.rs`,
/// resolve `path` against the working directory, unlike launching a program, and need no exec bit).
pub(crate) fn run_script_content(content: &str, scoped: bool, depth: usize) -> Result<(), String> {
    if depth >= MAX_SCRIPT_DEPTH {
        return Err(String::from("too many levels of scripts"));
    }
    let run_lines = || {
        for line in content.lines() {
            run_line_inner(line, depth + 1);
        }
    };
    if scoped {
        shell_state::frames().with_scope(|_frames| run_lines());
    } else {
        run_lines();
    }
    Ok(())
}

/// Reports one line of text as this segment's stderr: wherever the current frame's `stdio[2]` points
/// -- the console (and the UART transcript, for the test harness) by default, or a redirected file,
/// exactly like a program's own stderr. Used for syntax errors, a builtin's own errors and launch
/// failures alike, so e.g. `cmd 2> e` captures all of them the same way bash does. Starts a new
/// console row first if something else left the cursor mid-line.
pub fn shell_err(msg: &str) {
    match shell_state::stdio(2) {
        StdioBinding::Default => {
            uart_ensure_newline();
            uart_write(msg.as_bytes());
            uart_write(b"\n");
            // SAFETY: as `run`'s doc comment says of the statics it uses.
            unsafe {
                let console = static_mut_ref!(CONSOLE);
                if console.cursor().1 != 0 {
                    console.write_char('\n', FG, BG);
                }
                for c in msg.chars() {
                    console.write_char(c, FG, BG);
                }
                console.write_char('\n', FG, BG);
            }
        }
        StdioBinding::File(handle) => {
            let _ = files::write(handle, msg.as_bytes());
            let _ = files::write(handle, b"\n");
        }
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
                    run_line(&text);
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
