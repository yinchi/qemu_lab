//! General-purpose helpers with no natural home in a device- or state-specific module --
//! currently just the pair of macros every `static mut Option<T>` in this crate
//! (`platform/globals.rs`'s `BLK`/`GPU`/`CONSOLE`/`KEYBOARD`/`IDMAP`, `keyboard/keymap.rs`'s
//! `KEY_STATE`/`LOCK_STATE`, `keyboard/line.rs`'s `LINE`, `fs/blkio.rs`'s `VOL`) is reached through.

/// Expands to `&'static mut T`, given the bare name of a `static mut Option<T>` -- going through
/// `&raw mut` rather than naming the static directly in a `&mut` expression, the pattern the 2024
/// edition's `static_mut_refs` lint requires, since forming a raw pointer alone (unlike a
/// reference) never claims exclusivity. Panics if the static is still `None`.
///
/// Deliberately does *not* wrap its own expansion in `unsafe {}`: every call site still has to
/// write `unsafe` itself, exactly as it would calling a plain `unsafe fn`.
///
/// SAFETY (every call site): no other reference to the named static is alive at the same time --
/// each static this is used on documents why that holds where it's declared (`platform/globals.rs`,
/// `keyboard/keymap.rs`).
#[macro_export]
macro_rules! static_mut_ref {
    ($name:ident) => {
        (*(&raw mut $name)).as_mut().unwrap()
    };
}

/// Expands to `&'static T`, given the bare name of a `static mut Option<T>` --
/// Immutable counterpart to `static_mut_ref!`, going through `&raw const` instead of `&raw mut`.
#[macro_export]
macro_rules! static_ref {
    ($name:ident) => {
        (*(&raw const $name)).as_ref().unwrap()
    };
}
