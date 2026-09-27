//! Token-emission layer: turns one raw key event, together with `KeyState`/`LockState`,
//! into a `Token` -- one non-modifier key's evdev code, plus which of
//! Shift/Ctrl/Alt were held and whether CapsLock was on at the time.
//!
//! Deliberately a separate module from `keymap.rs`, sitting on top of it rather than inside it:
//! `KeyState`/`LockState` are pure state-tracking, updated the same way regardless of who reads
//! them; this is interpretation.
//!
//! `Token` deliberately doesn't commit to one meaning per key: it carries the raw evdev code
//! (see `keymap.rs`'s doc comment on evdev numbering) uniformly for *any* key -- a letter, Enter,
//! Left, F1, all the same shape -- rather than this layer inventing named variants per key or
//! per modifier combination. `Token::char()` resolves the printable character, if any, as a
//! derived property; everything else (what `Ctrl`+something should do, whether `Left` means
//! cursor movement or history) is left to whatever consumes these, since different consumers
//! want different things (a byte-stream shell wants a control byte for `Ctrl+C`; a raw-mode app
//! might want `Ctrl+Left` as a word-jump instead).
//!
//! Only reacts to genuine presses (`value == 1`); the caller is expected to not call this at all
//! for releases or auto-repeat (`value == 0`/`2`) -- auto-repeat support, if it's ever added, is
//! a separate concern layered on top of this, not this function re-emitting on its own.

use super::keymap::{KEY_NAMES, KeyState, LockState};

// The evdev codes something here or in `line.rs` names live in the shared `abi::keys` (the editor and the
// kernel's own key handling read them from one place); only the modifier and lock keys, which never
// become tokens, are named privately below.
use abi::keys::{
    KEY_KP0, KEY_KP1, KEY_KP2, KEY_KP3, KEY_KP4, KEY_KP5, KEY_KP6, KEY_KP7, KEY_KP8, KEY_KP9,
    KEY_KPASTERISK, KEY_KPDOT, KEY_KPMINUS, KEY_KPPLUS, KEY_KPSLASH,
};
use abi::keys::{KEY_SPACE, KEY_TAB, effective_code};

const KEY_LCTRL: u16 = 29;
const KEY_LSHIFT: u16 = 42;
const KEY_LALT: u16 = 56;
const KEY_CAPSLOCK: u16 = 58;
const KEY_NUMLOCK: u16 = 69;
const KEY_SCROLLLOCK: u16 = 70;
const KEY_RSHIFT: u16 = 54;
const KEY_RCTRL: u16 = 97;
const KEY_RALT: u16 = 100;

/// One non-modifier key press, plus the modifier/lock state at the time it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// The evdev `KEY_*` code of the key that was pressed -- never one of the modifier/lock
    /// codes themselves (see `emit`'s `is_modifier` check).
    pub code: u16,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// CapsLock's toggle state, not a held key -- captured here (rather than read fresh from
    /// `LockState` wherever `char()` is called) so a `Token` is a self-contained snapshot, the
    /// same way `shift`/`ctrl`/`alt` are.
    pub caps: bool,
    /// NumLock's toggle state, captured the same way as `caps`: it decides what a keypad key is
    /// (`char()`, `effective_code()`).
    pub num: bool,
    /// The key was already down: the keyboard's own auto-repeat, not a fresh press. A consumer that
    /// must act once per press (an editor's Save) ignores these; one that acts per keystroke (moving,
    /// typing, Backspace) takes them.
    pub repeat: bool,
}

/// Shifted variant for keys whose `KEY_NAMES` entry is already their unshifted character
/// (number row + punctuation) -- keeps `KEY_NAME_TABLE` the single source of truth for the base
/// character, this table only for what Shift turns it into.
const SHIFTED: &[(u16, char)] = &[
    (2, '!'),
    (3, '@'),
    (4, '#'),
    (5, '$'),
    (6, '%'),
    (7, '^'),
    (8, '&'),
    (9, '*'),
    (10, '('),
    (11, ')'),
    (12, '_'),
    (13, '+'),
    (26, '{'),
    (27, '}'),
    (39, ':'),
    (40, '"'),
    (41, '~'),
    (43, '|'),
    (51, '<'),
    (52, '>'),
    (53, '?'),
];

/// What a keypad key types: `* - + /` always, as on a PC, and the digits and the dot only with NumLock
/// on (with it off they navigate -- see `abi::keys::effective_code`). Shift is not consulted.
fn keypad_char(code: u16, num: bool) -> Option<char> {
    match code {
        KEY_KPASTERISK => Some('*'),
        KEY_KPMINUS => Some('-'),
        KEY_KPPLUS => Some('+'),
        KEY_KPSLASH => Some('/'),
        _ if !num => None,
        KEY_KP0 => Some('0'),
        KEY_KP1 => Some('1'),
        KEY_KP2 => Some('2'),
        KEY_KP3 => Some('3'),
        KEY_KP4 => Some('4'),
        KEY_KP5 => Some('5'),
        KEY_KP6 => Some('6'),
        KEY_KP7 => Some('7'),
        KEY_KP8 => Some('8'),
        KEY_KP9 => Some('9'),
        KEY_KPDOT => Some('.'),
        _ => None,
    }
}

/// Whether a `repeat` token may enter a queue that already holds `queued` tokens: it may not. A held
/// key repeats faster than a slow consumer (an editor redrawing a screen per key) can take, and every
/// repeat that waited in the queue would keep moving the cursor after the key was released -- so a
/// repeat is dropped unless the queue has been drained. A fresh press always enters.
pub fn admits(token: &Token, queued: usize) -> bool {
    !token.repeat || queued == 0
}

impl Token {
    /// The key as a program means it, with the numeric keypad resolved by NumLock -- the code to match
    /// commands and navigation on (`abi::keys::effective_code`); `code` stays the raw evdev code.
    pub fn effective_code(&self) -> u16 {
        effective_code(self.code, self.num)
    }

    /// The printable character this key resolves to, given Shift/CapsLock (and NumLock, for the
    /// keypad) -- `None` for keys with no character of their own (Enter, Left, F1, a keypad digit with
    /// NumLock off, ...) or that this layer doesn't (yet) resolve. Ctrl/Alt don't affect this; see this
    /// module's doc comment for why.
    ///
    /// SAFETY: KEY_NAMES must already be populated -- true from very early in `kernel_main`
    /// onward, same as every other reader of it (see its doc comment in `keymap.rs`).
    pub fn char(&self) -> Option<char> {
        match self.code {
            KEY_TAB => return Some('\t'),
            KEY_SPACE => return Some(' '),
            _ => {}
        }
        // The keypad's names ("Kp7") are multi-character, so `KEY_NAMES` would call them non-text keys.
        if let Some(c) = keypad_char(self.code, self.num) {
            return Some(c);
        }

        let name = unsafe { crate::static_ref!(KEY_NAMES) }.get_by_left(&self.code)?;
        let mut chars = name.chars();
        let first = chars.next()?;
        if chars.next().is_some() {
            // A multi-character name (F1, Left, Kp7, Enter, ...) -- not a text key.
            return None;
        }

        Some(if first.is_ascii_uppercase() {
            // Letter key: KEY_NAME_TABLE always spells these uppercase; actual case depends on
            // Shift XOR CapsLock, not the name's own case.
            if self.shift ^ self.caps {
                first
            } else {
                first.to_ascii_lowercase()
            }
        } else if self.shift {
            SHIFTED
                .iter()
                .find(|&&(c, _)| c == self.code)
                .map_or(first, |&(_, hi)| hi)
        } else {
            first
        })
    }
}

impl core::fmt::Display for Token {
    /// `^`/`M-` prefixes for Ctrl/Alt, then either the character (uppercased, bare, after `^` --
    /// classic caret notation always shows `Ctrl+t` and `Ctrl+T` the same way -- or quoted via
    /// `{:?}` otherwise, so whitespace stays visible) or, for a key with no character, its
    /// `KEY_NAMES` name in angle brackets (falling back to the bare code if `KEY_NAMES` somehow
    /// has no entry for it).
    ///
    /// SAFETY: see `char`'s doc comment; same requirement.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.ctrl {
            write!(f, "^")?;
        }
        if self.alt {
            write!(f, "M-")?;
        }
        match self.char() {
            Some(c) if self.ctrl => write!(f, "{}", c.to_ascii_uppercase()),
            Some(c) => write!(f, "{c:?}"),
            None => match unsafe { crate::static_ref!(KEY_NAMES) }.get_by_left(&self.code) {
                Some(name) => write!(f, "<{name}>"),
                None => write!(f, "<K{}>", self.code),
            },
        }
    }
}

/// Whether `code` is one of the modifier/lock keys themselves -- these never produce a `Token`
/// of their own: `KeyState`/`LockState` already track their held/toggled state continuously, so
/// a bare Shift press isn't an event a consumer should see, only something that changes how the
/// *next* key resolves.
fn is_modifier(code: u16) -> bool {
    matches!(
        code,
        KEY_LCTRL
            | KEY_RCTRL
            | KEY_LSHIFT
            | KEY_RSHIFT
            | KEY_LALT
            | KEY_RALT
            | KEY_CAPSLOCK
            | KEY_NUMLOCK
            | KEY_SCROLLLOCK
    )
}

/// Turns one key press into a token, given the current modifier/lock state -- or `None` if
/// `code` is one of the modifier/lock keys themselves (see `is_modifier`). `repeat` says the key was
/// already down (the caller knows: it read the held set before updating it).
pub fn emit(code: u16, keys: &KeyState, locks: &LockState, repeat: bool) -> Option<Token> {
    if is_modifier(code) {
        return None;
    }
    Some(Token {
        code,
        shift: keys.is_held(KEY_LSHIFT) || keys.is_held(KEY_RSHIFT),
        ctrl: keys.is_held(KEY_LCTRL) || keys.is_held(KEY_RCTRL),
        alt: keys.is_held(KEY_LALT) || keys.is_held(KEY_RALT),
        caps: locks.caps,
        num: locks.num,
        repeat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi::keys::{KEY_A, KEY_ENTER, KEY_HOME, KEY_KPENTER, KEY_LEFT};

    // None of these reach `KEY_NAMES` (host tests have no live kernel key-name state): the keypad is
    // resolved before that lookup, and a token's other fields are plain data.
    fn token(code: u16, num: bool, repeat: bool) -> Token {
        Token {
            code,
            shift: false,
            ctrl: false,
            alt: false,
            caps: false,
            num,
            repeat,
        }
    }

    #[test]
    fn keypad_digits_and_dot_type_only_with_numlock_on() {
        let digits = [
            (KEY_KP0, '0'),
            (KEY_KP1, '1'),
            (KEY_KP2, '2'),
            (KEY_KP3, '3'),
            (KEY_KP4, '4'),
            (KEY_KP5, '5'),
            (KEY_KP6, '6'),
            (KEY_KP7, '7'),
            (KEY_KP8, '8'),
            (KEY_KP9, '9'),
            (KEY_KPDOT, '.'),
        ];
        for (code, c) in digits {
            assert_eq!(keypad_char(code, true), Some(c));
            assert_eq!(keypad_char(code, false), None);
            assert_eq!(token(code, true, false).char(), Some(c));
        }
    }

    #[test]
    fn keypad_operators_type_whatever_numlock_says() {
        for num in [false, true] {
            assert_eq!(keypad_char(KEY_KPASTERISK, num), Some('*'));
            assert_eq!(keypad_char(KEY_KPMINUS, num), Some('-'));
            assert_eq!(keypad_char(KEY_KPPLUS, num), Some('+'));
            assert_eq!(keypad_char(KEY_KPSLASH, num), Some('/'));
            assert_eq!(token(KEY_KPPLUS, num, false).char(), Some('+'));
        }
    }

    #[test]
    fn shift_does_not_change_a_keypad_character() {
        let mut t = token(KEY_KP7, true, false);
        t.shift = true;
        assert_eq!(t.char(), Some('7'));
    }

    #[test]
    fn a_token_matches_on_its_effective_code() {
        assert_eq!(token(KEY_KP4, false, false).effective_code(), KEY_LEFT);
        assert_eq!(token(KEY_KP7, false, false).effective_code(), KEY_HOME);
        assert_eq!(
            token(KEY_KP4, true, false).effective_code(),
            abi::keys::KEY_4
        );
        assert_eq!(token(KEY_KPENTER, true, false).effective_code(), KEY_ENTER);
        assert_eq!(token(KEY_A, false, false).effective_code(), KEY_A);
        // the raw code is never rewritten
        assert_eq!(token(KEY_KP4, false, false).code, KEY_KP4);
    }

    #[test]
    fn emit_carries_the_lock_state_and_the_repeat_flag() {
        let keys = KeyState::new();
        let mut locks = LockState::new();
        assert!(locks.num, "NumLock starts on");
        let t = emit(KEY_KP4, &keys, &locks, false).unwrap();
        assert_eq!(
            (t.code, t.num, t.caps, t.repeat),
            (KEY_KP4, true, false, false)
        );

        locks.num = false;
        locks.caps = true;
        let t = emit(KEY_KP4, &keys, &locks, true).unwrap();
        assert_eq!((t.num, t.caps, t.repeat), (false, true, true));
    }

    #[test]
    fn emit_reads_the_held_modifiers() {
        let mut keys = KeyState::new();
        keys.set(KEY_LCTRL, true);
        keys.set(KEY_RALT, true);
        let t = emit(KEY_A, &keys, &LockState::new(), false).unwrap();
        assert_eq!((t.shift, t.ctrl, t.alt), (false, true, true));
    }

    #[test]
    fn modifier_and_lock_keys_never_become_tokens() {
        let keys = KeyState::new();
        let locks = LockState::new();
        for code in [
            KEY_LCTRL,
            KEY_RCTRL,
            KEY_LSHIFT,
            KEY_RSHIFT,
            KEY_LALT,
            KEY_RALT,
            KEY_CAPSLOCK,
            KEY_NUMLOCK,
            KEY_SCROLLLOCK,
        ] {
            assert!(emit(code, &keys, &locks, false).is_none());
            assert!(emit(code, &keys, &locks, true).is_none());
        }
    }

    #[test]
    fn a_fresh_press_always_enters_the_queue() {
        let press = token(KEY_A, true, false);
        for queued in [0, 1, 255] {
            assert!(admits(&press, queued));
        }
    }

    #[test]
    fn a_repeat_enters_only_an_empty_queue() {
        let repeat = token(KEY_A, true, true);
        assert!(admits(&repeat, 0));
        assert!(!admits(&repeat, 1));
        assert!(!admits(&repeat, 255));
    }
}
