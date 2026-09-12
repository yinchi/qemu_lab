//! Turns raw UART bytes into [`Token`]s: a pure, `no_std`/`alloc`-free byte
//! stream parser with no dependency on the UART, the timer hardware, or any
//! particular consumer's notion of a line buffer or cursor -- which is what
//! makes it reusable unchanged by any future consumer (Stage 11's shell,
//! Stage 12's editor) that wants the same key recognition but a completely
//! different response to it.
//!
//! # Correctness policy
//!
//! Every possible byte (or timeout) this parser could ever receive falls
//! into one of three tiers, in decreasing order of what's guaranteed:
//!
//! 1. **Genuine input** -- a real key, alone or with standard modifiers,
//!    that an actual keyboard can produce. Recognized properly, up to a
//!    point: given a specific, meaningful [`Token`] as far as this parser
//!    has been built out to go (not every real combination has one yet --
//!    see the [`Token::Char`]/`;`-parameter note below for a named
//!    example -- but the ones it does claim to handle must be correct).
//! 2. **Weird-but-real combinations** -- genuine keyboard input whose wire
//!    encoding collides with something else, or that hasn't been given a
//!    specific meaning (`Ctrl+Alt+Shift+O`, which a real terminal sends as
//!    `ESC` + `Ctrl-O` and this parser resolves as `Alt('\u{0f}')`, is a
//!    confirmed example). No obligation to classify these meaningfully --
//!    only to terminate: emit some number of tokens (possibly zero) and
//!    return to [`ParserState::Ground`].
//! 3. **Inputs no keyboard could organically produce** -- garbled,
//!    scripted, or adversarial byte streams. No obligation on *what* comes
//!    out, only that parsing still can't get stuck.
//!
//! Tiers 2 and 3 are the same guarantee in practice: every non-`Ground`
//! state's `match` is exhaustive and includes a `None` (timeout) arm, so
//! nothing this parser does on its own can prevent an eventual return to
//! `Ground`. That said, this guarantee is only half of a contract -- this
//! module can promise "if you eventually call [`parse`] with `None`, I will
//! resolve," but it cannot by itself guarantee that call actually happens.
//! The other half belongs to whichever caller wires this up to real
//! hardware (`main.rs`, today): it must always arm a bounded timer on any
//! transition to a non-`Ground` state, and reliably feed `None` back when
//! that timer fires.

/// Where we are in recognizing a possible escape sequence. Every non-`Ground`
/// state has a bounded timer armed against it (see `ESC_TIMEOUT_MS`) -- there
/// is no state this parser can get stuck in indefinitely.
#[derive(Clone, Copy, PartialEq)]
pub enum ParserState {
    Ground,
    SawEsc,
    /// Saw `ESC O` -- the "SS3" prefix xterm's default mode uses for F1-F4
    /// (`ESC O P`/`Q`/`R`/`S`), a genuinely different escape family from the
    /// `ESC [ ...` CSI form everything else here uses.
    SawEscO,
    SawEscBracket,
    /// Saw `ESC [` followed by one or more digits -- packed one hex nibble
    /// per digit (`value = (value << 4) | nibble`) rather than accumulated
    /// as a decimal number. `0` maps to nibble `0xa` instead of `0x0`, so
    /// `0x0` unambiguously means "no digit here yet": that's what makes
    /// [`digits_to_ascii`] able to recover the exact original digit string
    /// (leading zeros included) straight back out of the packed value later,
    /// for the timeout-reconstruction case below. A bare `<<` can't panic
    /// or need overflow handling the way arithmetic accumulation would --
    /// an implausibly long digit run just silently shifts its earliest
    /// digits out the top of the `u32`, which is fine, since anything we
    /// actually act on is at most two digits.
    SawEscBracketDigits(u32),
}

/// Maps an ASCII digit byte to the hex nibble `SawEscBracketDigits` packs it
/// as: `0` becomes `0xa` (never `0x0`), so `0x0` is free to mean "no digit
/// here yet" when reconstructing the original digits back out.
fn digit_nibble(c: u8) -> u32 {
    match c {
        b'0' => 0xa,
        _ => (c - b'0') as u32,
    }
}

/// Recovers the original digit characters from a `SawEscBracketDigits`
/// value, in the order they were typed (leading zeros included), by
/// reading it one hex nibble at a time from the most significant end and
/// skipping the untouched (`0x0`) nibbles above wherever accumulation
/// actually started. Returns how many of `out`'s bytes were filled in.
fn digits_to_ascii(value: u32, out: &mut [u8; 8]) -> usize {
    let mut n = 0;
    let mut started = false;
    for shift in (0..8).rev() {
        let nibble = (value >> (shift * 4)) & 0xf;
        if nibble == 0 && !started {
            continue;
        }
        started = true;
        out[n] = if nibble == 0xa {
            b'0'
        } else {
            b'0' + nibble as u8
        };
        n += 1;
    }
    n
}

/// Maps a C0 control byte (`0x01..=0x1a`) to the letter held with Ctrl to
/// produce it (`0x01` = Ctrl-A = `'a'`, ..., `0x1a` = Ctrl-Z = `'z'`).
fn ctrl_letter(b: u8) -> char {
    (b + 0x60) as char
}

#[derive(Clone, Copy)]
pub enum Token {
    Char(char),
    // Payload not yet read anywhere: recognized by the parser, but nothing
    // in main.rs binds an action to which letter/function key it was yet
    // (see dispatch_tokens's Token::Ctrl(_) | Token::Fn(_) arm). Scoped to
    // just these two variants, not the whole enum, so dead-code analysis
    // still catches anything genuinely forgotten added here later.
    #[allow(dead_code)]
    Ctrl(char),
    #[allow(dead_code)]
    Fn(u8),
    Alt(char),
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Tab,
    Backspace,
    Escape,
    Enter,
}

/// A small, fixed-capacity, stack-allocated buffer of `Token`s, standing in
/// for a `Vec<Token>`. `parse` runs on every single received byte, so
/// heap-allocating a fresh `Vec` each time would mean real allocator churn
/// on a hot path for what's almost always zero or one token -- and the
/// worst case (a fully-reconstructed broken digit sequence: `^[` + up to 8
/// digits + one trailing character) is a fixed, small, known bound, so a
/// stack array is a strictly better fit than a heap collection here.
pub struct Tokens {
    items: [Option<Token>; Self::CAP],
    len: usize,
}

impl Tokens {
    /// `^` + `[` + up to 8 reconstructed digits + one trailing character.
    const CAP: usize = 11;

    const fn new() -> Self {
        Self {
            items: [None; Self::CAP],
            len: 0,
        }
    }

    fn push(&mut self, t: Token) {
        self.items[self.len] = Some(t);
        self.len += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = Token> + '_ {
        self.items[..self.len].iter().map(|t| t.unwrap())
    }
}

/// Builds a `Tokens` from a literal list, the same way `vec![...]` would
/// build a `Vec` -- just onto the stack instead of the heap.
macro_rules! tokens {
    () => {
        Tokens::new()
    };
    ($($t:expr),+ $(,)?) => {{
        let mut ts = Tokens::new();
        $(ts.push($t);)+
        ts
    }};
}

/// Handles a broken escape sequence by converting the accumulated digits
/// into a sequence of `Token`s representing the original characters typed.
fn handle_broken_sequence(digits: u32) -> Tokens {
    let mut out = tokens![Token::Char('^'), Token::Char('[')];
    let mut digits_out = [0u8; 8];
    let n = digits_to_ascii(digits, &mut digits_out);
    for &b in &digits_out[..n] {
        out.push(Token::Char(b as char));
    }
    out
}

/// Parse a single input byte (`Some`) or timeout (`None`).
pub fn parse(input: Option<u8>, state: ParserState) -> (Tokens, ParserState) {
    match state {
        ParserState::Ground => match input {
            None => (tokens![], ParserState::Ground),
            Some(b'\x1b') => (tokens![], ParserState::SawEsc),
            Some(b'\t') => (tokens![Token::Tab], ParserState::Ground),
            Some(b'\x08') => (tokens![Token::Backspace], ParserState::Ground),
            Some(b'\x7f') => (tokens![Token::Delete], ParserState::Ground),
            Some(b'\r') => (tokens![Token::Enter], ParserState::Ground),
            Some(b'\n') => (tokens![Token::Enter], ParserState::Ground),
            // Every other C0 control byte: Ctrl-A through Ctrl-Z, minus the
            // ones already claimed a specific meaning above (Tab, Backspace,
            // CR/LF, Escape -- Rust's first-match-wins arm ordering is what
            // lets this range arm sit after them safely).
            Some(c @ 0x01..=0x1a) => (tokens![Token::Ctrl(ctrl_letter(c))], ParserState::Ground),
            Some(c) => (tokens![Token::Char(c as char)], ParserState::Ground),
        },
        ParserState::SawEsc => match input {
            None => (tokens![Token::Escape], ParserState::Ground),
            Some(b'[') => (tokens![], ParserState::SawEscBracket),
            Some(b'O') => (tokens![], ParserState::SawEscO),
            _ => (
                tokens![Token::Alt(input.unwrap() as char)],
                ParserState::Ground,
            ),
        },
        ParserState::SawEscO => match input {
            None => (tokens![Token::Alt('O')], ParserState::Ground),
            Some(b'P') => (tokens![Token::Fn(1)], ParserState::Ground),
            Some(b'Q') => (tokens![Token::Fn(2)], ParserState::Ground),
            Some(b'R') => (tokens![Token::Fn(3)], ParserState::Ground),
            Some(b'S') => (tokens![Token::Fn(4)], ParserState::Ground),
            _ => (
                tokens![Token::Alt('O'), Token::Char(input.unwrap() as char)],
                ParserState::Ground,
            ),
        },
        ParserState::SawEscBracket => match input {
            None => (tokens![Token::Alt('[')], ParserState::Ground),
            Some(b'A') => (tokens![Token::ArrowUp], ParserState::Ground),
            Some(b'B') => (tokens![Token::ArrowDown], ParserState::Ground),
            Some(b'C') => (tokens![Token::ArrowRight], ParserState::Ground),
            Some(b'D') => (tokens![Token::ArrowLeft], ParserState::Ground),
            Some(b'F') => (tokens![Token::End], ParserState::Ground),
            Some(b'H') => (tokens![Token::Home], ParserState::Ground),
            Some(c @ b'0'..=b'9') => {
                let digit = digit_nibble(c);
                (tokens![], ParserState::SawEscBracketDigits(digit))
            }
            _ => (
                tokens![Token::Alt('['), Token::Alt(input.unwrap() as char)],
                ParserState::Ground,
            ),
        },
        ParserState::SawEscBracketDigits(digits) => match input {
            // Continue accumulating -- without this arm, a second digit
            // falls into the catch-all below and is treated as breaking the
            // sequence, which defeats the entire point of packing multiple
            // digits into `digits` in the first place (no two-digit code,
            // e.g. any of F5-F12, could ever be recognized).
            Some(c @ b'0'..=b'9') => {
                let digit = digit_nibble(c);
                (
                    tokens![],
                    ParserState::SawEscBracketDigits((digits << 4) | digit),
                )
            }
            Some(b'~') => {
                // Handle sequences like '^[<digits>~'
                // where <digits> is a sequence of digit characters
                // encoded in hex (A = '0', 1-9 = '1'-'9')
                match digits {
                    1 => (tokens![Token::Home], ParserState::Ground),
                    2 => (tokens![Token::Insert], ParserState::Ground),
                    3 => (tokens![Token::Delete], ParserState::Ground),
                    4 => (tokens![Token::End], ParserState::Ground),
                    5 => (tokens![Token::PageUp], ParserState::Ground),
                    6 => (tokens![Token::PageDown], ParserState::Ground),

                    // For some reason the Fn keys skip over some numbers
                    0x15 => (tokens![Token::Fn(5)], ParserState::Ground),
                    0x17 => (tokens![Token::Fn(6)], ParserState::Ground),
                    0x18 => (tokens![Token::Fn(7)], ParserState::Ground),
                    0x19 => (tokens![Token::Fn(8)], ParserState::Ground),
                    0x2A => (tokens![Token::Fn(9)], ParserState::Ground),
                    0x21 => (tokens![Token::Fn(10)], ParserState::Ground),
                    0x23 => (tokens![Token::Fn(11)], ParserState::Ground),
                    0x24 => (tokens![Token::Fn(12)], ParserState::Ground),

                    // Unrecognized digit sequence for '^[<digits>~'
                    _ => {
                        let mut out = handle_broken_sequence(digits);
                        out.push(Token::Char('~'));
                        (out, ParserState::Ground)
                    }
                }
            }
            Some(b';') => {
                // `;` introduces a second parameter -- real terminals use
                // this for modifier-shifted CSI sequences, e.g. Shift+Right
                // as `ESC[1;2C`, or Alt+F1 as `ESC[1;3P`. Deliberately Tier
                // 3, not Tier 1: these are genuine keyboard combinations,
                // but not ones we give specific tokens to right now --
                // termination (reconstruct what was consumed, plus this
                // byte, and return to Ground) is all that's required here.
                let mut out = handle_broken_sequence(digits);
                out.push(Token::Char(';'));
                (out, ParserState::Ground)
            }

            // Catch-all for any other input while in SawEscBracketDigits
            // state (a real byte or a timeout) -- abort and return to the
            // ground state. Not matching a literal `0x17` here specifically
            // (as an earlier version of this did): that byte is genuine
            // Ctrl-W input as far as this state is concerned, not a timeout
            // signal -- timeouts are `None`, unambiguously, handled by the
            // same arm below.
            Some(c) => {
                let mut out = handle_broken_sequence(digits);
                out.push(Token::Char(c as char));
                (out, ParserState::Ground)
            }
            None => {
                let out = handle_broken_sequence(digits);
                (out, ParserState::Ground)
            }
        },
    }
}
