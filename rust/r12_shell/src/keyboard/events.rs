//! The keyboard-event-to-token step shared by both consumers of `Keyboard::poll()`: the IRQ
//! path (`main.rs`'s `handle_keyboard_irq`, for the shell's own prompt) and the `read(0)`
//! syscall path (`stdin.rs`, for a running program), which can't rely on the IRQ at all -- see
//! `stdin.rs`. Sharing this keeps the two from drifting apart on what counts as a keypress.

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
/// `KEYBOARD`, from either `handle_keyboard_irq` (which can't be re-entered) or a syscall (IRQs
/// masked for the whole program) -- so at most one caller is ever live.
pub fn token_for(event: InputEvent) -> Option<Token> {
    if event.event_type != EV_KEY {
        return None;
    }

    // value: 0 = released, 1 = pressed, 2 = auto-repeat (treated as still-pressed).
    let down = event.value != 0;

    // SAFETY: see this function's doc comment.
    let key_changed = unsafe { static_mut_ref!(KEY_STATE) }.set(event.code, down);
    // Lock keys flip on press only -- LockState::apply ignores release/auto-repeat itself.
    // SAFETY: see this function's doc comment.
    unsafe { static_mut_ref!(LOCK_STATE) }.apply(event.code, event.value);

    if down && key_changed {
        // SAFETY: see this function's doc comment.
        unsafe { tokens::emit(event.code, static_ref!(KEY_STATE), static_ref!(LOCK_STATE)) }
    } else {
        None
    }
}
