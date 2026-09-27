//! The keyboard-event-to-token step of `queue::drain_keyboard`, the queue's one producer -- called from the
//! IRQ handler and from a blocked `read(0)` (`stdin.rs`), which can't rely on the IRQ (syscalls run with
//! IRQs masked). Having one place decide what counts as a keypress keeps the two from drifting apart.

use super::keymap::{KEY_STATE, LOCK_STATE};
use super::tokens::{self, Token};
use crate::drivers::virtio::input::{EV_KEY, InputEvent};
use crate::{static_mut_ref, static_ref};

/// Updates the held-key set and lock-key toggles from `event`, and returns the `Token` it
/// produces, if any -- only a genuine down-edge of a non-modifier key does.
///
/// Gating on `event.value == 1` alone isn't enough: `KeyState::set`'s changed-flag is what's
/// actually robust to a held key resending its press, whether the transport tags that a
/// `value == 2` auto-repeat or -- as observed on this virtio-input/QEMU setup -- just resends
/// plain `value == 1` events with no distinct repeat tag at all.
///
/// SAFETY (of the `static_mut_ref!`/`static_ref!` uses below): only ever called while draining
/// `KEYBOARD`, from either the IRQ handler (which can't be re-entered) or a syscall (IRQs masked for
/// its whole duration, so the two never overlap) -- so at most one caller is ever live.
pub fn token_for(event: InputEvent) -> Option<Token> {
    if event.event_type != EV_KEY {
        return None;
    }

    // value: 0 = released, 1 = pressed, 2 = auto-repeat (treated as still-pressed).
    let down = event.value != 0;

    // A press of a key that is already held is the keyboard's own auto-repeat, whether the transport
    // tags it `value == 2` or (as observed here) resends a plain `value == 1`; asked *before* the held
    // set is updated, since afterwards the two cases look alike. A code past the held set's range is
    // never "held", so an exotic key is neither a press nor a repeat.
    // SAFETY: see this function's doc comment.
    let was_held = unsafe { static_ref!(KEY_STATE) }.is_held(event.code);
    // SAFETY: see this function's doc comment.
    let key_changed = unsafe { static_mut_ref!(KEY_STATE) }.set(event.code, down);
    // A lock key flips on a fresh press only: a resent press (its auto-repeat) must not toggle it again.
    if key_changed {
        // SAFETY: see this function's doc comment.
        unsafe { static_mut_ref!(LOCK_STATE) }.apply(event.code, event.value);
    }

    if down && (key_changed || was_held) {
        // SAFETY: see this function's doc comment.
        unsafe {
            tokens::emit(
                event.code,
                static_ref!(KEY_STATE),
                static_ref!(LOCK_STATE),
                was_held,
            )
        }
    } else {
        None
    }
}
