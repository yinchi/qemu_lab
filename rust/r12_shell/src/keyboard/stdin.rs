//! `read(0)`: hands a running program one finished line of typed input at a time.
//!
//! This can't reuse the IRQ path the shell's own prompt uses. `process::run_program` masks every
//! DAIF bit for a program's whole time at EL0, so no keyboard IRQ ever arrives while a program
//! runs -- nothing would ever complete a line for it to read. Instead `read(0)` blocks inside
//! the syscall, draining `KEYBOARD` directly (the device's queue fills regardless of the mask)
//! through the same `events::token_for` -> `LineBuffer::feed` path the prompt uses, so Backspace
//! is absorbed here exactly as it is there -- a program only ever sees a finished line's
//! printable bytes plus a trailing newline, like a real tty's cooked mode.
//!
//! Ctrl+D on an empty line is end-of-file (`read` returns 0), so a program reading until EOF
//! (`cat` with no arguments) has a way to stop. On a non-empty line it does nothing.

use alloc::vec::Vec;

use super::events;
use super::line::{LINE, LineEvent};
use super::tokens::KEY_D;
use crate::console::{BG, FG, show_row};
use crate::platform::globals::{CONSOLE, GPU, KEYBOARD};
use crate::platform::uart::uart_write;
use crate::static_mut_ref;

// The rest of the last line handed out, when the program's buffer was smaller than the line: the
// bytes, and how far into them the program has read.
//
// SAFETY (every access): single core, syscalls run with IRQs masked -- nothing else touches
// either one.
static mut PENDING: Vec<u8> = Vec::new();
static mut PENDING_POS: usize = 0;

/// Forgets any leftover line -- called at the start of each launch, so one program's unread
/// input can't leak into the next.
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
    // SAFETY: syscalls run with IRQs masked, so `handle_keyboard_irq` can't be running -- these
    // are the same statics it uses, with the same at-most-one-user guarantee (see platform/globals.rs).
    let (console, gpu, kb, line) = unsafe {
        (
            static_mut_ref!(CONSOLE),
            static_mut_ref!(GPU),
            static_mut_ref!(KEYBOARD),
            static_mut_ref!(LINE),
        )
    };

    // Start typing on a fresh row if the program left its cursor mid-line.
    if console.cursor().1 != 0 {
        console.write_char('\n', FG, BG);
    }
    let row = console.cursor().0;

    loop {
        // Clears the device's interrupt line so it doesn't keep re-asserting (unserviced, since
        // IRQs are masked) and cutting every `wfe` below short.
        kb.ack_interrupt();

        while let Some(event) = kb.poll() {

            // Decode the keyboard event into a token, if possible.
            let Some(token) = events::token_for(event) else {
                continue;
            };

            // Handle Ctrl+D (end-of-file) if the line is empty.
            if token.ctrl && token.code == KEY_D && line.as_str().is_empty() {
                return None;
            }

            // Feed the token into the line buffer and handle the resulting event.
            match line.feed(token) {
                Some(LineEvent::Changed) => {
                    show_row(console, row, "", line.as_str());
                    gpu.flush();
                }
                Some(LineEvent::Finished(text)) => {
                    console.write_char('\n', FG, BG);
                    gpu.flush();
                    uart_write(text.as_bytes());
                    uart_write(b"\n");
                    return Some(text.into_bytes());
                }
                None => {}
            }
        }

        // SAFETY: plain `wfe`, no memory or register effects -- see blk.rs's identical waits.
        unsafe { core::arch::asm!("wfe") };
    }
}
