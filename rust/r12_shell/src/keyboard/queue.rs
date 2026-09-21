//! The keyboard's token queue: the one place key presses wait between the interrupt that learns of them
//! and whoever reads them. The producer is `drain_keyboard`, which moves whatever the device has into
//! the queue; the consumers are the shell's loop (`shell::run`) and a blocked `read(0)` (`stdin.rs`),
//! both of which feed what they pop to the line discipline.
//!
//! Nothing here draws, launches or blocks, so it is safe to run from the interrupt handler -- which
//! is all the handler does with the keyboard. That is what lets programs run with interrupts enabled:
//! a key pressed while one runs just waits in the queue, in order, for the next reader, and no shell
//! code can be re-entered from an interrupt. (Before Step 5 the shell ran *inside* the interrupt, so
//! every interrupt had to stay masked while a program ran.)
//!
//! Who touches what: `drain_keyboard` runs from the IRQ handler, or from `read(0)` inside a syscall, where
//! IRQs are masked, so there is only ever one producer at a time. The shell's loop pops with IRQs
//! on, so `pop` masks them for the moment it takes. The queue is reader-agnostic on purpose: whoever
//! owns the keyboard pops it (later, a raw-mode editor -- Stage 13 -- or a foreground program --
//! Stage 19).

use super::events::token_for;
use super::ring_buffer::RingBuffer;
use super::tokens::Token;
use crate::arch::irq::{wait_for_interrupt_unless, without_irqs};
use crate::platform::globals::KEYBOARD;
use crate::platform::uart::{uart_ensure_newline, uart_write};
use crate::static_mut_ref;

/// How many key presses can wait. A test build (`testhooks`) uses a tiny queue so the overflow path can
/// be exercised by typing a few dozen keys instead of hundreds.
#[cfg(not(feature = "testhooks"))]
const CAPACITY: usize = 256;
#[cfg(feature = "testhooks")]
const CAPACITY: usize = 16;

/// SAFETY (every access): see this module's doc comment.
static mut TOKENS: RingBuffer<Token, CAPACITY> = RingBuffer::new();

/// Whether the overflow note for the current burst has been printed -- one note per burst, not one per
/// dropped key. Cleared once the queue has been emptied.
static mut OVERFLOW_NOTED: bool = false;

/// Moves every pending event from the keyboard device into the queue, as tokens. If the queue is full
/// the newest presses are dropped, and the UART gets one note per burst.
///
/// Callable from the IRQ handler and from `read(0)`'s wait loop: neither can be interrupted by the other.
#[allow(clippy::deref_addrof)]
pub fn drain_keyboard() {
    // SAFETY: see this module's doc comment; KEYBOARD is populated before its interrupt is enabled.
    let keyboard = unsafe { static_mut_ref!(KEYBOARD) };
    // Clears the device's interrupt line so it doesn't keep re-asserting.
    keyboard.ack_interrupt();
    while let Some(event) = keyboard.poll() {
        let Some(token) = token_for(event) else {
            continue;
        };
        // SAFETY: see this module's doc comment.
        let (tokens, noted) = unsafe { (&mut *(&raw mut TOKENS), &mut *(&raw mut OVERFLOW_NOTED)) };
        if tokens.push(token).is_err() && !*noted {
            *noted = true;
            uart_ensure_newline();
            uart_write(b"[keyboard: input queue full, further keys dropped]\n");
        }
    }
}

/// Takes the oldest waiting key press, if there is one.
#[allow(clippy::deref_addrof)]
pub fn pop() -> Option<Token> {
    // IRQs off for the moment it takes: the interrupt handler pushes into the same queue.
    without_irqs(|| {
        // SAFETY: see this module's doc comment.
        let (tokens, noted) = unsafe { (&mut *(&raw mut TOKENS), &mut *(&raw mut OVERFLOW_NOTED)) };
        let token = tokens.pop();
        if tokens.is_empty() {
            *noted = false; // the next burst gets its own note
        }
        token
    })
}

/// Sleeps until a key press may have arrived, returning at once if the queue isn't empty. Returns with
/// IRQs on, and possibly with nothing to pop (any interrupt wakes it).
#[allow(clippy::deref_addrof)]
pub fn wait_for_token() {
    wait_for_interrupt_unless(|| {
        // SAFETY: IRQs are masked here (see `wait_for_interrupt_unless`).
        !unsafe { &*(&raw const TOKENS) }.is_empty()
    });
}
