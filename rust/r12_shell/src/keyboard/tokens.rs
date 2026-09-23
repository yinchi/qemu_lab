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

// Exported for a future raw-mode consumer (Stage 13) to match `Token::code` against -- nothing
// in this stage's canonical line loop has a defined action for Escape yet.
#[allow(dead_code)]
pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
pub const KEY_SPACE: u16 = 57;
// Ctrl+D on an empty line is `read(0)`'s end-of-file -- see `stdin.rs`.
pub const KEY_D: u16 = 32;

// Step 12 (`Stage12.md`): cursor movement, history, and readline-style Ctrl combinations --
// `Mode::Prompt` only, see `line.rs`'s "Token handling by mode" table.
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_DELETE: u16 = 111;
/// Ctrl+A: move to the start of the line.
pub const KEY_A: u16 = 30;
/// Ctrl+E: move to the end of the line.
pub const KEY_E: u16 = 18;
/// Ctrl+U: erase from the cursor to the start of the line (both modes -- see `line.rs`).
pub const KEY_U: u16 = 22;
/// Ctrl+K: erase from the cursor to the end of the line.
pub const KEY_K: u16 = 37;

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

impl Token {
    /// The printable character this key resolves to, given Shift/CapsLock -- `None` for keys
    /// with no character of their own (Enter, Left, F1, ...) or that this layer doesn't (yet)
    /// resolve. Ctrl/Alt don't affect this; see this module's doc comment for why.
    ///
    /// SAFETY: KEY_NAMES must already be populated -- true from very early in `kernel_main`
    /// onward, same as every other reader of it (see its doc comment in `keymap.rs`).
    pub fn char(&self) -> Option<char> {
        match self.code {
            KEY_TAB => return Some('\t'),
            KEY_SPACE => return Some(' '),
            _ => {}
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
/// `code` is one of the modifier/lock keys themselves (see `is_modifier`).
pub fn emit(code: u16, keys: &KeyState, locks: &LockState) -> Option<Token> {
    if is_modifier(code) {
        return None;
    }
    Some(Token {
        code,
        shift: keys.is_held(KEY_LSHIFT) || keys.is_held(KEY_RSHIFT),
        ctrl: keys.is_held(KEY_LCTRL) || keys.is_held(KEY_RCTRL),
        alt: keys.is_held(KEY_LALT) || keys.is_held(KEY_RALT),
        caps: locks.caps,
    })
}
