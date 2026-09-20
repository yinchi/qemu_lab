//! A keymap/modifier-state layer, as scoped by the roadmap: interprets raw key events into two
//! kinds of derived state:
//!
//! - `KeyState`: which keys are currently *held*
//! - `LockState`: which lock keys (CapsLock/NumLock/ScrollLock) are currently *toggled on* --
//!   flips once per press, ignoring release.
//!
//! Both types of state ignore auto-repeat events (timer-based key repeats are more reliable
//! anyway if we need to support such events).
//!
//! This module also owns the live global instances of both (`KEY_STATE`/`LOCK_STATE`), plus a
//! code<->name `BiMap` (`KEY_NAMES`) used to display them -- all reached by `main.rs` via
//! `utils.rs`'s `static_mut_ref!`/`static_ref!` macros.
//!
//! `code` values are Linux evdev `KEY_*` constants (`input-event-codes.h`) -- confirmed against
//! QEMU's `virtio-keyboard-device`, which reports codes in that numbering, not raw PS/2 scancodes
//! or USB HID usage IDs.

use alloc::string::String;
use core::fmt::Write as _;

use bimap::BiMap;

/// `KEY_MAX` (`0x2ff`), evdev's own ceiling on `EV_KEY` codes -- shared with mouse/
/// joystick/gamepad button codes, which is why it's so much higher than any real keyboard key.
const KEY_MAX: u16 = 0x2ff;

/// Sized to `KEY_MAX + 1` -- evdev's *entire* `EV_KEY` code space, not just the keys this file
/// happens to name.
const MAX_CODE: usize = KEY_MAX as usize + 1;

/// `(code, name)` pairs for every `KEY_*` code this file names. Codes under `MAX_CODE` with no
/// entry here still get tracked and displayed -- just as a bare `K<code>` instead of a name.
/// Feeds `build_key_names` below; kept as a plain data table (not a `match`) specifically so the
/// same pairs are available for lookup in *either* direction, not just code -> name -- e.g.
/// resolving "Kp0".."Kp9"/"F1".."F12" by name, which aren't contiguous code ranges.
const KEY_NAME_TABLE: &[(u16, &str)] = &[
    (1, "Esc"),
    (2, "1"),
    (3, "2"),
    (4, "3"),
    (5, "4"),
    (6, "5"),
    (7, "6"),
    (8, "7"),
    (9, "8"),
    (10, "9"),
    (11, "0"),
    (12, "-"),
    (13, "="),
    (14, "Backspace"),
    (15, "Tab"),
    (16, "Q"),
    (17, "W"),
    (18, "E"),
    (19, "R"),
    (20, "T"),
    (21, "Y"),
    (22, "U"),
    (23, "I"),
    (24, "O"),
    (25, "P"),
    (26, "["),
    (27, "]"),
    (28, "Enter"),
    (29, "LCtrl"),
    (30, "A"),
    (31, "S"),
    (32, "D"),
    (33, "F"),
    (34, "G"),
    (35, "H"),
    (36, "J"),
    (37, "K"),
    (38, "L"),
    (39, ";"),
    (40, "'"),
    (41, "`"),
    (42, "LShift"),
    (43, "\\"),
    (44, "Z"),
    (45, "X"),
    (46, "C"),
    (47, "V"),
    (48, "B"),
    (49, "N"),
    (50, "M"),
    (51, ","),
    (52, "."),
    (53, "/"),
    (54, "RShift"),
    (55, "KpAsterisk"),
    (56, "LAlt"),
    (57, "Space"),
    (58, "CapsLock"),
    (59, "F1"),
    (60, "F2"),
    (61, "F3"),
    (62, "F4"),
    (63, "F5"),
    (64, "F6"),
    (65, "F7"),
    (66, "F8"),
    (67, "F9"),
    (68, "F10"),
    (69, "NumLock"),
    (70, "ScrollLock"),
    (71, "Kp7"),
    (72, "Kp8"),
    (73, "Kp9"),
    (74, "KpMinus"),
    (75, "Kp4"),
    (76, "Kp5"),
    (77, "Kp6"),
    (78, "KpPlus"),
    (79, "Kp1"),
    (80, "Kp2"),
    (81, "Kp3"),
    (82, "Kp0"),
    (83, "KpDot"),
    (87, "F11"),
    (88, "F12"),
    (96, "KpEnter"),
    (97, "RCtrl"),
    (98, "KpSlash"),
    (100, "RAlt"),
    (102, "Home"),
    (103, "Up"),
    (104, "PageUp"),
    (105, "Left"),
    (106, "Right"),
    (107, "End"),
    (108, "Down"),
    (109, "PageDown"),
    (110, "Insert"),
    (111, "Delete"),
    (125, "LMeta"),
    (126, "RMeta"),
    (127, "Compose"),
];

/// Builds the code<->name `BiMap` from `KEY_NAME_TABLE`. Not `const` -- `BiMap` is hashmap-backed
/// (needs `alloc`, and hashing isn't available at compile time) -- so this runs once at boot,
/// same as everything else `main.rs`'s `kernel_main` sets up once and hands to a static; see
/// `KEY_NAMES` below.
pub fn build_key_names() -> BiMap<u16, &'static str> {
    KEY_NAME_TABLE.iter().copied().collect()
}

/// Tracks which keys are currently held, indexed directly by keycode.
pub struct KeyState {
    held: [bool; MAX_CODE],
    /// The last key that was pressed (if still held), else None.
    /// In other words, `None` may indicate that no key is currently held, or that the last
    /// key pressed has been released.
    last_held: Option<u16>,
}

impl KeyState {
    pub const fn new() -> Self {
        Self {
            held: [false; MAX_CODE],
            last_held: None,
        }
    }

    /// Applies a press (`down = true`) or release (`down = false`) for `code`.
    ///
    /// Returns whether the held set actually changed, so callers can skip a redundant redraw --
    /// this is what makes auto-repeat (a `value == 2` event, reported as another press) a no-op:
    /// the key is already recorded as held, so `set` returns `false` and nothing redraws.
    pub fn set(&mut self, code: u16, down: bool) -> bool {
        let idx = code as usize;

        // Early return if the index is out of bounds or the key's state hasn't changed.
        // Ensures correct last_held handling by only updating it when a key's state actually
        // changes.
        if idx >= MAX_CODE || self.held[idx] == down {
            return false;
        }

        // Update the held state for this key.
        self.held[idx] = down;

        // Update the last_held field if this is a key-down or a key-up of the last_held key.
        if down {
            // Key-down
            self.last_held = Some(code);
        } else if self.last_held == Some(code) {
            // Key-up of the last_held key. Clear the last_held field.
            // We do not keep track of previously held keys beyond the last one; this
            // is consistent with most systems where releasing the last key pressed stops
            // any auto-repeat behavior.
            self.last_held = None;
        }

        // Return success status.
        true
    }

    /// A space-separated list of the currently held keys' names, in keycode order (not press
    /// order -- simplest to compute, and the demo has no need to distinguish the two).
    ///
    /// SAFETY: KEY_NAMES must already be populated -- true from very early in `kernel_main`
    /// onward (see `main.rs`), well before this is ever called.
    #[allow(dead_code)]
    pub fn describe(&self) -> String {
        let mut s = String::new();
        for (code, &held) in self.held.iter().enumerate() {
            if !held {
                continue;
            }
            if !s.is_empty() {
                s.push(' ');
            }
            // SAFETY: see this method's doc comment.
            match unsafe { crate::static_ref!(KEY_NAMES) }.get_by_left(&(code as u16)) {
                Some(name) => s.push_str(name),
                None => {
                    let _ = write!(s, "K{code}");
                }
            }
        }
        if s.is_empty() {
            s.push_str("(none)");
        }
        s
    }

    /// Returns whether `code` is currently held. Read-only access for layers built on top of
    /// this one -- e.g. `tokens.rs`'s token-emission layer, which needs to know Shift/Ctrl's
    /// current state without this module duplicating that interpretation itself.
    pub fn is_held(&self, code: u16) -> bool {
        self.held.get(code as usize).copied().unwrap_or(false)
    }

    /// Returns the last held key, if any.
    #[allow(dead_code)]
    pub fn describe_last_held(&self) -> String {
        // SAFETY: see this method's doc comment.
        match self.last_held {
            Some(code) => match unsafe { crate::static_ref!(KEY_NAMES) }.get_by_left(&code) {
                Some(name) => String::from(*name),
                None => String::from("K{code}"),
            },
            None => String::from("(none)"),
        }
    }
}

/// Tracks which lock keys are currently toggled on.
///
/// Deliberately a separate type from `KeyState`, which tracks held keys and has no concept of
/// a toggle state.
pub struct LockState {
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

impl LockState {
    pub const fn new() -> Self {
        Self {
            caps: false,
            num: false,
            scroll: false,
        }
    }

    /// Applies one raw input event. Only a genuine press (`value == 1`, never a release or
    /// auto-repeat) of a lock key flips anything. Returns whether a bit actually flipped, so
    /// callers can skip a redundant redraw the same way `KeyState::set` does.
    ///
    /// Identifies lock keys by looking their code up in `KEY_NAMES` and matching the name,
    /// rather than three separate `KEY_CAPSLOCK`/`KEY_NUMLOCK`/`KEY_SCROLLLOCK` code constants --
    /// `KEY_NAME_TABLE` is already the one source of truth for what each code is called, so this
    /// avoids a second, independent place that has to agree with it on the same three codes.
    ///
    /// SAFETY: KEY_NAMES must already be populated -- true well before this is ever called, same
    /// as `KeyState::describe` (see its doc comment).
    pub fn apply(&mut self, code: u16, value: u32) -> bool {
        if value != 1 {
            return false;
        }
        // SAFETY: see this method's doc comment.
        let name = unsafe { crate::static_ref!(KEY_NAMES) }
            .get_by_left(&code)
            .copied();
        let bit = match name {
            Some("CapsLock") => &mut self.caps,
            Some("NumLock") => &mut self.num,
            Some("ScrollLock") => &mut self.scroll,
            _ => return false,
        };
        *bit = !*bit;
        true
    }

    /// A space-separated list of the currently-on lock keys' names.
    #[allow(dead_code)]
    pub fn describe(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.caps, "Caps"),
            (self.num, "Num"),
            (self.scroll, "Scroll"),
        ] {
            if on {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(name);
            }
        }
        if s.is_empty() {
            s.push_str("(none)");
        }
        s
    }
}

// Live global instances, populated once by `main.rs`'s `kernel_main` before the keyboard's GIC
// line (KEYBOARD_SPI, back in main.rs alongside BLK_SPI -- IRQ-routing plumbing, not keymap
// state) is ever enabled -- from that point on, only `irq_handler` (and what it calls) ever
// touches them, and at most one `irq_handler` invocation runs at a time (single core, IRQs
// masked for its duration). Reached via `utils.rs`'s `static_mut_ref!`/`static_ref!` macros,
// same as BLK/GPU/CONSOLE/KEYBOARD -- see those macros' doc comments for the SAFETY contract
// every call site is relying on; it's identical here, just restated there once instead of once
// per accessor function the way this file used to.
pub static mut KEY_STATE: Option<KeyState> = None;
pub static mut LOCK_STATE: Option<LockState> = None;

// KEY_NAMES is populated even earlier than the two above -- right at the top of `kernel_main`,
// before anything else -- since `KeyState::describe` (called from `main.rs`'s
// `show_keyboard_state`, itself called both from `kernel_main` directly and from every
// `handle_keyboard_irq` redraw) needs it from the very first call onward. Read-only after that
// one-time population; nothing ever writes it again, so unlike KEY_STATE/LOCK_STATE only
// `static_ref!` (never `static_mut_ref!`) is ever used on it.
pub static mut KEY_NAMES: Option<BiMap<u16, &'static str>> = None;
