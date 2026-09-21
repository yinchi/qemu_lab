//! `read(0)`: hands a running program one finished line of typed input at a time.
//!
//! The program is blocked inside the syscall, with IRQs masked, so no keyboard interrupt can reach the
//! queue while it waits: `read_line` drains the device into the queue itself (`queue::drain_keyboard`,
//! the same producer the interrupt handler uses), pops what is there and feeds it to the line
//! discipline (`line_discipline.rs`) -- the one the shell's prompt uses -- so Backspace is absorbed here
//! exactly as it is there. A program only ever sees a finished line's printable bytes plus a trailing
//! newline, like a real tty's cooked mode. Keys typed before the program asked for input wait in the
//! queue and are read like any other, in order.
//!
//! Ctrl+D on an empty line is end-of-file (`read` returns 0), so a program reading until EOF
//! (`cat` with no arguments) has a way to stop. On a non-empty line it does nothing.

use alloc::vec::Vec;

use super::line_discipline::{LINE_DISCIPLINE, LineOutcome, Mode};
use super::queue;
use crate::platform::globals::{CONSOLE, GPU};
use crate::static_mut_ref;

// SAFETY (every access): single core, syscalls run with IRQs masked -- nothing else touches
// either one.
/// The line buffer for a partially consumed line (the most recent one from the keyboard).
static mut PENDING: Vec<u8> = Vec::new();
/// The current read position within the pending line buffer.
static mut PENDING_POS: usize = 0;

/// Forgets any leftover line -- called at the start of each launch, so one program's unread
/// input can't leak into the next.
#[allow(clippy::deref_addrof)]
pub fn reset() {
    // SAFETY: see PENDING.
    unsafe {
        (*(&raw mut PENDING)).clear();
        PENDING_POS = 0;
    }
}

/// Fills `buf` from the current line, blocking for a new one if it's used up. Returns the number
/// of bytes copied, or `0` at end-of-file (Ctrl+D on an empty line).
pub fn read(buf: &mut [u8]) -> isize {
    if buf.is_empty() {
        return 0;
    }
    // SAFETY: see PENDING.
    #[allow(clippy::deref_addrof)]
    let (pending, pos) = unsafe { (&mut *(&raw mut PENDING), &mut *(&raw mut PENDING_POS)) };

    if *pos >= pending.len() {
        let Some(mut line) = read_line() else {
            return 0;
        };
        line.push(b'\n');
        *pending = line;
        *pos = 0;
    }

    let n = buf.len().min(pending.len() - *pos);
    buf[..n].copy_from_slice(&pending[*pos..*pos + n]);
    *pos += n;
    n as isize
}

/// Blocks until Enter finishes a line (returned without its newline), echoing what's typed at
/// the console's current row as it goes; `None` on Ctrl+D with nothing typed.
fn read_line() -> Option<Vec<u8>> {
    // SAFETY: the shell's loop is inside `launch` (it called us, through the program), so nothing
    // else uses these statics -- and the interrupt handler never does (see `queue.rs`).
    let (console, gpu, discipline) = unsafe {
        (
            static_mut_ref!(CONSOLE),
            static_mut_ref!(GPU),
            static_mut_ref!(LINE_DISCIPLINE),
        )
    };

    // Start typing on a fresh row if the program left its cursor mid-line; nothing goes before the
    // line.
    discipline.begin(console, "", Mode::Canonical);

    loop {
        // IRQs are masked inside a syscall, so the device's events wait there until we fetch them.
        queue::drain_keyboard();

        while let Some(token) = queue::pop() {
            match discipline.handle(token, console) {
                LineOutcome::Ignored => {}
                LineOutcome::Edited => gpu.flush(),
                LineOutcome::Finished(text) => {
                    gpu.flush();
                    return Some(text.into_bytes());
                }
                LineOutcome::EndOfFile => return None,
            }
        }

        // Nothing typed yet: sleep until an interrupt line goes up (a masked interrupt still wakes
        // `wfi`); the loop then fetches it.
        // SAFETY: plain `wfi`, no memory or register effects.
        unsafe { core::arch::asm!("wfi") };
    }
}
