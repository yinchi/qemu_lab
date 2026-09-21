//! Keyboard input, from raw key events to lines: `events` turns a driver event into a `Token`
//! (`keymap` tracks which keys are held and which locks are on, `tokens` resolves a key to a
//! character), `line` holds the text of the line being typed, `line_discipline` is the line discipline that
//! edits, echoes and finishes it (shared by the shell's prompt and `read(0)`), and `stdin` hands a
//! running program's `read(0)` a finished line. The layer above (`shell`) decides what a finished line means; this one may
//! draw the line being typed on the `console`, but the console never calls back into it.

pub mod events;
pub mod keymap;
pub mod line;
pub mod line_discipline;
pub mod stdin;
pub mod tokens;
