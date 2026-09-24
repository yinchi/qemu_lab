//! The shell: everything above the syscalls. `lexer` and `syntax` turn a typed line into a `Pipeline`
//! of commands with their redirections, `builtins` runs the few commands that change the shell's own
//! state, `launch` starts programs, and `run` is the read-eval loop `kernel_main` ends in and never
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
use crate::exec::shell_state::{self, Frames, Stdio};
use crate::fs::blkio::VOL;
use crate::fs::files::{self, FileRef};
use crate::keyboard::line_discipline::{LINE_DISCIPLINE, LineDiscipline, LineOutcome, Mode};
use crate::keyboard::queue;
use crate::platform::globals::{CONSOLE, GPU};
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::{static_mut_ref, static_ref};
use launch::launch;
use syntax::{Redirection, Segment};

/// The prompt shown before the line being typed -- fixed text with no relation to the line's own
/// content, so it's structurally impossible for any editing key (which only ever touches the line
/// buffer, see `keyboard/line.rs`) to erase into or through it.
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

/// Runs one typed line: parses it (`syntax.rs`) and executes it -- a builtin (`builtins.rs`), a
/// program (`launch.rs`), or a multi-stage pipeline (`run_pipeline`) -- reporting whatever went
/// wrong via `shell_err`. A blank line or a comment does nothing, quietly, as in any shell.
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
    // The returned status isn't used here -- `exit N` (our stand-in for `$?` until Stage 17, see
    // `ROADMAP.md`'s Stage 17 section) is printed by `run_segment_with` itself, while its
    // `with_stdio` scope is still active, so a redirected stderr captures it exactly like the rest
    // of a failing command's own output. Once Stage 17 adds real `$?`, this is where the returned
    // value would be saved instead (no redirect concern for a plain variable write, so it need not
    // move back inside any scope) -- the return chain underneath doesn't change either way.
    if let [segment] = pipeline.as_slice() {
        run_segment(segment, depth);
    } else {
        run_pipeline(&pipeline, depth);
    }
}

/// Runs one segment with no pipe bindings -- the plain, single-command case, always reporting its
/// own exit status. A thin wrapper over `run_segment_with`; see it for what actually happens.
fn run_segment(segment: &Segment, depth: usize) -> Option<i32> {
    run_segment_with(segment, [None, None, None], true, depth)
}

/// Runs one segment: opens its redirections in order -- left to right, each already in effect for
/// the ones after it, so `2>&1 > f` and `> f 2>&1` differ -- then the command itself, all under one
/// `with_stdio` scope seeded with `overrides`. That scope is why a builtin's own state change (`cd`)
/// still sticks under a redirect, why a failed redirect leaves exactly the earlier ones of the same
/// line in effect, and why the error message for that failure (and any launch error) is itself
/// subject to whichever redirects already succeeded -- `cmd 2> e < missing` reports the missing-file
/// error into `e`, not the console. The files a redirect opened are held by the frame's bindings, so they
/// are closed when the scope ends and restores the streams, committing anything written -- unless a
/// program somehow still held one.
///
/// `overrides` is `[None, None, None]` for a plain command (`run_segment`); a pipeline stage
/// (`run_pipeline`) instead seeds it with the pipe's own binding, which the segment's redirects
/// below then correctly layer on top of via `apply_redirect`'s plain overwrite -- "the pipe binds
/// before the stage's own redirects," with no new mechanism beyond what redirection already does.
///
/// `report` is whether this call should print `exit {code}` (our stand-in for `$?` until Stage 17)
/// for a nonzero result -- `true` for a plain command, `true` only for a pipeline's *last* stage
/// (`run_pipeline`). The print happens *inside* the `with_stdio` scope, not after it returns, so a
/// redirected stderr captures it exactly like the rest of a failing command's own output -- moving
/// it outside was tried and breaks `cmd 2> e`'s existing, already-tested behavior.
///
/// Returns `None` if no program actually ran (a builtin, a script, or any of `launch`'s own
/// already-reported failures) or `Some(code)` if one did -- see `launch`'s doc comment.
fn run_segment_with(
    segment: &Segment,
    overrides: [Option<Stdio>; 3],
    report: bool,
    depth: usize,
) -> Option<i32> {
    shell_state::frames().with_stdio(overrides, |frames| {
        for redir in &segment.redirs {
            if !apply_redirect(frames, redir) {
                return None; // shell_err already reported; the command does not run
            }
        }
        let status = run_command(&segment.argv, depth);
        if report
            && let Some(code) = status
            && code != 0
        {
            shell_err(&format!("exit {code}"));
        }
        status
    })
}

/// Opens and binds one redirection on `frames`' top frame. The bindings hold the file open, so it is
/// closed when the last of them is dropped (the scope ending, or a later redirect of the same stream
/// replacing it). Returns whether it succeeded; on failure it has already reported the error
/// via `shell_err`.
fn apply_redirect(frames: &mut Frames, redir: &Redirection) -> bool {
    match redir {
        Redirection::In(path) => match open_redirect_target(path, false, false) {
            Ok(file) => {
                frames.top_mut().stdio[0] = Stdio::File(file);
                true
            }
            Err(msg) => {
                shell_err(&msg);
                false
            }
        },
        Redirection::Out { fd, path, append } => match open_redirect_target(path, true, *append) {
            Ok(file) => {
                frames.top_mut().stdio[*fd as usize] = Stdio::File(file);
                true
            }
            Err(msg) => {
                shell_err(&msg);
                false
            }
        },
        Redirection::Dup { fd, target } => {
            // The same open file, shared: closing one of the two fds leaves the other working.
            let binding = frames.top().stdio[*target as usize].clone();
            frames.top_mut().stdio[*fd as usize] = binding;
            true
        }
    }
}

/// Resolves `path` against the working directory and opens it for a redirection, in bash's wording
/// on failure. The file outlives whatever program runs under the redirect because the frame's binding
/// holds a reference to it, whatever that program does to its own fds.
fn open_redirect_target(path: &str, write: bool, append: bool) -> Result<FileRef, String> {
    let abspath = shell_state::absolute(path).map_err(|e| format!("{path}: {}", errmsg(e)))?;
    files::open(&abspath, write, append).map_err(|e| format!("{path}: {}", errmsg(e)))
}

/// Runs a segment's command -- once its redirections (if any) are already bound -- as a builtin or a
/// program. Empty `argv` (a stage of only redirections, `> f`) runs nothing: POSIX still creates the
/// file. A builtin never contributes a numeric status (none of `cd`/`source`/`.`/`sh` have one,
/// matching their existing behavior of never printing `exit N`); only a program launched via
/// `launch` can return `Some(code)`.
fn run_command(argv: &[String], depth: usize) -> Option<i32> {
    let name = argv.first()?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    if builtins::is_builtin(name) {
        if let Err(message) = builtins::run(name, &argv[1..], depth) {
            shell_err(&message);
        }
        None
    } else {
        // SAFETY: as `run`'s doc comment says of the statics it uses.
        let vol = unsafe { static_ref!(VOL) };
        launch(vol, &argv, depth)
    }
}

/// Prefix for pipeline temp file names -- a leading dot, distinct from anything a user is likely to
/// type by hand. Not itself the safety mechanism against colliding with a real file (`next_pipe_path`
/// is); just a first line of defense that makes an accidental collision unlikely in the first place.
const PIPE_PREFIX: &str = "/tmp/.pipe";

/// How many names `next_pipe_path` will skip past an existing file before giving up -- guards
/// against a pathological `/tmp/`, not a realistic case.
const MAX_PIPE_NAME_ATTEMPTS: usize = 1000;

/// Monotonic counter for pipeline temp file names: one for the shell's *whole lifetime*, deliberately
/// not reset per pipeline -- a pipeline stage can itself be a script that runs its own pipeline
/// internally, and if each pipeline reset its own counter to 0, an inner pipe's temp file could
/// collide with an outer, still-alive one. One counter that only ever increments guarantees every
/// simultaneously-alive temp file has a unique name regardless of nesting depth.
///
/// SAFETY (every access): single core, and the shell runs with interrupts enabled but nothing else
/// ever touches this.
static mut PIPE_COUNTER: usize = 0;

/// Picks the next unused pipeline temp file path: `/tmp/.pipeN` for the lowest `N` (from
/// `PIPE_COUNTER`) that `files::stat` says doesn't already exist. `open`'s own create-or-truncate
/// has no `O_EXCL`, so checking first is what actually prevents clobbering a file a user happens to
/// have sitting at that name -- safe to check-then-create with no race, since this shell is
/// single-threaded and strictly sequential (nothing else can claim the name in between). `Err` if
/// nothing works out within `MAX_PIPE_NAME_ATTEMPTS` tries.
fn next_pipe_path() -> Result<String, String> {
    // SAFETY: see PIPE_COUNTER.
    #[allow(clippy::deref_addrof)]
    let counter = unsafe { &mut *(&raw mut PIPE_COUNTER) };
    for _ in 0..MAX_PIPE_NAME_ATTEMPTS {
        let path = format!("{PIPE_PREFIX}{counter}");
        *counter += 1;
        if files::stat(&path).is_err() {
            return Ok(path);
        }
    }
    Err(String::from("cannot create a unique temp file"))
}

/// Removes every temp file in `paths`, ignoring errors -- there is nothing more useful to do about a
/// failed cleanup, and the pipeline is already finishing or aborting either way.
fn cleanup_temps(paths: &[String]) {
    for path in paths {
        let _ = files::unlink(path, false);
    }
}

/// Runs a multi-stage pipeline (`Stage12.md`'s Step 11): each stage's stdout feeds the next stage's
/// stdin through a temp file under `/tmp/`, POSIX-style. Every stage always runs, even if an earlier
/// one failed, faulted, or wasn't found -- only a *setup* failure (disk full, `/tmp` missing) aborts
/// the rest, never a stage's own failure. The pipe is bound before a stage's own redirections (see
/// `run_segment_with`), so `a > f | b` sends `a`'s output to `f`, and `b` sees empty input. The
/// pipeline's status is the *last* stage's: only its call to `run_segment_with` passes `report:
/// true`, so `exit N` (if the last stage's code is nonzero) is the only one that can ever print,
/// from inside that stage's own `with_stdio` scope -- see `run_segment_with`'s doc comment for why
/// it can't be printed here instead.
///
/// Temp paths are all allocated before any stage runs and removed on every exit from this function.
/// Each pipe file is held by the binding the stage runs under, so it is closed as soon as the stage's
/// `run_segment_with` returns, committing the write side's size to disk before the next stage reads it.
fn run_pipeline(pipeline: &[Segment], depth: usize) -> Option<i32> {
    let n = pipeline.len();
    let mut temps: Vec<String> = Vec::new();
    for _ in 0..n - 1 {
        match next_pipe_path() {
            Ok(path) => temps.push(path),
            Err(msg) => {
                shell_err(&msg);
                cleanup_temps(&temps);
                return None;
            }
        }
    }

    let mut last_status = None;
    for (i, segment) in pipeline.iter().enumerate() {
        let mut overrides = [None, None, None];

        if i > 0
            && let Err(msg) = open_redirect_target(&temps[i - 1], false, false)
                .map(|file| overrides[0] = Some(Stdio::File(file)))
        {
            shell_err(&msg);
            cleanup_temps(&temps);
            return None;
        }
        if i < n - 1
            && let Err(msg) = open_redirect_target(&temps[i], true, false)
                .map(|file| overrides[1] = Some(Stdio::File(file)))
        {
            shell_err(&msg);
            cleanup_temps(&temps);
            return None;
        }

        last_status = run_segment_with(segment, overrides, i == n - 1, depth);
    }

    cleanup_temps(&temps);
    last_status
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
        Stdio::Default => {
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
        Stdio::File(file) => {
            let _ = files::write(&file, msg.as_bytes());
            let _ = files::write(&file, b"\n");
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
                // `Partial` (Ctrl+D on a non-empty line) is `Mode::Canonical`-only -- see
                // `line_discipline.rs` -- and this loop always runs in `Mode::Prompt`, so it can
                // never actually happen here; matched anyway since `LineOutcome` has no wildcard
                // arm elsewhere in this crate either.
                LineOutcome::Ignored | LineOutcome::EndOfFile | LineOutcome::Partial(_) => {}
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
