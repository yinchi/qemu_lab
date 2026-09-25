//! Splits a command line into tokens: words, `|`, and redirection operators. Replaces `shlex`, which
//! threw away whether a word was quoted -- so `echo "|"` would have looked like a pipe, and `a>b`
//! stayed one word.
//!
//! The rules are POSIX's, as far as this shell goes:
//! - A word is what lies between blanks. Single quotes are fully literal. Inside double quotes everything is
//!   literal except `$` expansions (below) and that a backslash escapes `"`, `\`, `$` and a backtick (so `"\$"`
//!   is `$`); any other backslash there is itself. An unquoted backslash makes the next character literal.
//! - `$NAME`, `${NAME}` and `$?` are expansions, in an unquoted word and inside double quotes, but not inside single
//!   quotes or after a backslash. A name is `[A-Za-z_][A-Za-z0-9_]*`; `$NAME` takes the longest one. The lexer only
//!   records them: a `Word` is a list of `Part`s, and `expand.rs` gives them values. A `$` followed by anything
//!   else (a digit, another `$`, a blank, the end of the line) is an ordinary character, since there are no
//!   positional parameters or process ids to name; `${` that is not `${NAME}` is a `bad substitution`.
//! - `#` starts a comment only at the start of a word: `echo a#b` prints `a#b`, `echo a #b` prints `a`.
//! - A word that starts with an unquoted `NAME=` (a valid name, then `=`) is marked as a possible assignment;
//!   whether it is one depends on where it stands in the command (`syntax.rs`: before the command word).
//!   Quoting or escaping any part of the name, or a name that is not valid (`1A=x`, `a-b=x`), makes it an
//!   ordinary word.
//! - `|`, `<`, `>`, `>>` and `>&` end a word and are operators. An unquoted, unescaped word that is just the
//!   digit `1` or `2` (or `0` before `<`), *immediately* followed by an operator, is that operator's file
//!   descriptor number (`2>err`, `2>>err`, `2>&1`); anywhere else digits are ordinary text (`a2>x` is the word
//!   `a2` and `>x`; `echo 2 >x` echoes `2`; a quoted `"2">x` is a word; `3>x` is the word `3` and `>x`).
//! - backtick, `*`, `?`, `~`, `{` and `}` are ordinary characters: nothing is expanded or globbed yet.
//! - `;`, `&`, `(`, `)` and here-documents (`<<`) are not supported, and are refused rather than taken for text.
//!
//! Built on `peg` (a PEG parser-generator: `#[macro] peg::parser!{}` below expands into an ordinary
//! recognizer, `no_std`-capable with `default-features = false`), rather than hand-written character
//! matching -- this grammar is `rust/docs/shell.ebnf`'s `token`/`redir_op` productions directly, not a
//! from-scratch reimplementation of them. Each `{? Err("...") }` block sits at the exact alternative
//! that detects a specific problem (an unterminated quote, a bare `;`, `<<`); `classify` below turns
//! the sentinel string that block chose back into the `LexError` it stands for, a flat conversion, not
//! a second attempt at working out what went wrong.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::exec::frame_stack::is_valid_name;

/// A redirection operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirOp {
    /// `<`
    In,
    /// `>`
    Out,
    /// `>>`
    Append,
    /// `>&`: duplicate an output descriptor onto another (`2>&1`).
    DupOut,
}

impl RedirOp {
    /// The operator as typed, for messages.
    pub fn text(self) -> &'static str {
        match self {
            RedirOp::In => "<",
            RedirOp::Out => ">",
            RedirOp::Append => ">>",
            RedirOp::DupOut => ">&",
        }
    }
}

/// One piece of a word. What quoting the text had is gone by the time it is a `Lit` (adjacent text, quoted
/// or not, is one `Lit`), since only an expansion's result cares: a quoted one is never split into fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Text taken as written. `""` is a `Lit` of nothing, which keeps an empty word from vanishing.
    Lit(String),
    /// `$NAME` or `${NAME}`. `quoted` is whether it sat inside double quotes.
    Var { name: String, quoted: bool },
    /// `$?`.
    Status { quoted: bool },
}

/// A word as typed: its parts in order, never empty, with no two `Lit`s adjacent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub parts: Vec<Part>,
    /// If the word starts with an unquoted `NAME=`, the length of `NAME`: the first `Lit` then begins with it.
    assign: Option<usize>,
}

impl Word {
    /// A word of the text `text`, taken literally.
    pub fn literal(text: &str) -> Word {
        Word::from_parts(vec![Part::Lit(String::from(text))])
    }

    /// A word of `parts`, normalized: adjacent `Lit`s joined. Never an assignment (only the lexer says so).
    pub fn from_parts(parts: Vec<Part>) -> Word {
        let mut merged: Vec<Part> = Vec::with_capacity(parts.len());
        for part in parts {
            match (merged.last_mut(), part) {
                (Some(Part::Lit(text)), Part::Lit(more)) => text.push_str(&more),
                (_, part) => merged.push(part),
            }
        }
        Word { parts: merged, assign: None }
    }

    /// The word as an assignment, if it could be one: the name and the value, a word of what follows the `=`
    /// (empty text if nothing does).
    pub fn assignment(&self) -> Option<(&str, Word)> {
        let n = self.assign?;
        let Some(Part::Lit(first)) = self.parts.first() else { return None };
        let mut parts = Vec::with_capacity(self.parts.len());
        if first.len() > n + 1 {
            parts.push(Part::Lit(String::from(&first[n + 1..])));
        }
        parts.extend(self.parts[1..].iter().cloned());
        if parts.is_empty() {
            parts.push(Part::Lit(String::new()));
        }
        Some((&first[..n], Word::from_parts(parts)))
    }

    /// The text of a word that has no expansion in it, or `None` if it has one.
    pub fn as_literal(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [Part::Lit(text)] => Some(text),
            _ => None,
        }
    }
}

impl From<&str> for Word {
    fn from(text: &str) -> Word {
        Word::literal(text)
    }
}

/// The word as it reads back, for messages: text as is, an expansion as `$NAME` or `$?` (quotes are not kept).
impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.parts {
            match part {
                Part::Lit(text) => f.write_str(text)?,
                Part::Var { name, .. } => write!(f, "${name}")?,
                Part::Status { .. } => f.write_str("$?")?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Word(Word),
    /// `|`
    Pipe,
    /// A redirection operator and the descriptor number typed right before it, if any.
    Redir { fd: Option<u8>, op: RedirOp },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LexError {
    /// A quote or a trailing backslash with nothing after it.
    Unterminated(&'static str),
    /// Syntax this shell doesn't have: `;`, `&`, `(`, `)`, `<<`, `<&`.
    Unsupported(&'static str),
    /// `${` that is not followed by a name and `}`.
    BadSubstitution,
}

peg::parser! {
    grammar shell_lexer() for str {
        rule blank()
            = [' ' | '\t' | '\x0B' | '\x0C' | '\r' | '\n']

        // Characters excluded from an unquoted word: either an operator (its own rule elsewhere) or
        // refused outright (";", "&", "(", ")" have no rule anywhere else in this grammar).
        rule unquoted_start()
            = !['"' | '\'' | '\\' | '|' | '<' | '>' | '#' | ';' | '&' | '(' | ')'
                | ' ' | '\t' | '\x0B' | '\x0C' | '\r' | '\n'] [_]
        // "#" may continue a word already started but is excluded from unquoted_start, so it can
        // never start one -- at a fresh position only `comment` matches "#".
        rule unquoted_tail_char() = unquoted_start() / "#"

        rule name_start() = ['A'..='Z' | 'a'..='z' | '_']
        rule name() -> &'input str = $(name_start() ['A'..='Z' | 'a'..='z' | '0'..='9' | '_']*)
        rule braced() -> &'input str = "${" n:name() "}" { n }
        // An expansion. The second alternative is the sentinel for a `${` that is not `${NAME}`; the
        // callers below refuse to take that `$` as ordinary text (`!"${"`), so it is what the parse fails on.
        // `&braced()` first so that a `${a-b}` failing at the `-` does not put its own, further, failure
        // ahead of the sentinel's in the error.
        rule expansion(quoted: bool) -> Part
            = &braced() n:braced() { Part::Var { name: n.to_string(), quoted } }
            / "${" {? Err("bad-substitution") }
            / "$?" { Part::Status { quoted } }
            / "$" n:name() { Part::Var { name: n.to_string(), quoted } }

        rule unquoted_item() -> Part
            = expansion(false)
            / !"${" c:$(unquoted_start()) { Part::Lit(c.to_string()) }
        rule unquoted_tail_item() -> Part
            = expansion(false)
            / !"${" c:$(unquoted_tail_char()) { Part::Lit(c.to_string()) }
        rule unquoted() -> Vec<Part> = f:unquoted_item() r:unquoted_tail_item()* {
            let mut parts = vec![f];
            parts.extend(r);
            parts
        }

        rule squote_char() = !"'" [_]
        rule squoted() -> Vec<Part>
            = "'" s:$(squote_char()*) "'" { vec![Part::Lit(s.to_string())] }
            / "'" squote_char()* {? Err("unterminated-quote") }
        // No escaping inside single quotes.

        rule dquote_escape() -> char = "\\" c:['"' | '\\' | '$' | '`'] { c }
        rule dquote_item() -> Part
            = expansion(true)
            / c:dquote_escape() { Part::Lit(c.to_string()) }
            / !"\"" !"${" c:$([_]) { Part::Lit(c.to_string()) }
        // A backslash not immediately before one of the four specials falls through to the catch-all
        // and is kept, as itself, by the *next* dquote_item -- pushing a lone '\\' and letting the
        // following character (or end of input) be handled normally on the next iteration, rather
        // than needing a dedicated "backslash but not before a special" rule.
        rule dquoted() -> Vec<Part>
            = "\"" ps:dquote_item()* "\"" { if ps.is_empty() { vec![Part::Lit(String::new())] } else { ps } }
            / "\"" dquote_item()* {? Err("unterminated-quote") }

        rule escape_piece() -> Vec<Part>
            = "\\" c:$([_]) { vec![Part::Lit(c.to_string())] }
            / "\\" {? Err("unterminated-escape") }
        // Outside any quote, a backslash escapes exactly the next character, whatever it is; with
        // nothing after it (end of input), that's an error distinct from an unterminated quote.

        rule piece() -> Vec<Part> = dquoted() / squoted() / escape_piece()
        rule tail() -> Vec<Part>
            = p:piece() rest:unquoted_tail_item()* { let mut parts = p; parts.extend(rest); parts }

        // A word is unquoted text with zero or more (piece, then more text) pairs after it, or -- if
        // it starts with a piece instead -- one or more of those same pairs.
        rule word_token() -> Word
            = u:unquoted() ts:tail()* {
                // Only the leading unquoted run `u` can hold an unquoted `NAME=`.
                let assign = assignment_name_len(&Word::from_parts(u.clone()).parts);
                let mut parts = u;
                parts.extend(ts.into_iter().flatten());
                let mut word = Word::from_parts(parts);
                word.assign = assign;
                word
              }
            / ts:tail()+ { Word::from_parts(ts.into_iter().flatten().collect()) }

        rule comment() = "#" [_]*

        // A bare "0"/"1"/"2" pairs with an operator only when immediately followed by the one it
        // belongs to; a longer digit run, or a digit not immediately adjacent to an operator, is left
        // for `word_token` -- so every one of these is tried *before* word_token, or word_token would
        // claim a lone eligible digit as its own word before the operator rule ever saw it.
        rule digit0() -> char = ['0']
        rule digit12() -> char = ['1' | '2']

        // "<<" and "<&" are refused outright, regardless of what (if anything) precedes them --
        // listed before in_redir, or in_redir would consume just the fd digit and the first "<",
        // leaving a stray second character to be mis-parsed as its own token.
        rule op_heredoc() -> Token = digit0()? "<<" {? Err("op-heredoc") }
        rule op_dupin() -> Token = digit0()? "<&" {? Err("op-dupin") }
        // The lookahead matters, not just the ordering above: `{? Err(...) }` failing only makes
        // *that* alternative backtrack, it doesn't abort the parse -- without this, "<<" would fail
        // op_heredoc and then silently succeed here as a bare "<", one character short of the truth,
        // with the stray second "<" or "&" left to be mis-parsed as its own token right afterward.
        rule in_redir() -> Token
            = d:digit0()? "<" !['<' | '&'] { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::In } }

        // append_redir/dup_redir listed before out_redir for the same reason: ">" is a strict prefix
        // of both, so trying it first would steal their match.
        rule append_redir() -> Token
            = d:digit12()? ">>" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::Append } }
        rule dup_redir() -> Token
            = d:digit12()? ">&" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::DupOut } }
        rule out_redir() -> Token
            = d:digit12()? ">" { Token::Redir { fd: d.map(|c| c as u8 - b'0'), op: RedirOp::Out } }

        rule op_semicolon() -> Token = ";" {? Err("op-semicolon") }
        rule op_amp() -> Token = "&" {? Err("op-amp") }
        rule op_lparen() -> Token = "(" {? Err("op-lparen") }
        rule op_rparen() -> Token = ")" {? Err("op-rparen") }

        rule pipe() -> Token = "|" { Token::Pipe }

        rule token() -> Token
            = op_heredoc()
            / op_dupin()
            / in_redir()
            / append_redir()
            / dup_redir()
            / out_redir()
            / op_semicolon()
            / op_amp()
            / op_lparen()
            / op_rparen()
            / pipe()
            / w:word_token() { Token::Word(w) }

        pub rule line() -> Vec<Token>
            = ts:(t:token() {Some(t)} / blank() {None} / comment() {None})* {
                ts.into_iter().flatten().collect()
              }
    }
}

/// The length of the name if `parts` (an unquoted run) starts with `NAME=`.
fn assignment_name_len(parts: &[Part]) -> Option<usize> {
    let Some(Part::Lit(text)) = parts.first() else { return None };
    let eq = text.find('=')?;
    is_valid_name(&text[..eq]).then_some(eq)
}

/// Splits `line` into tokens.
pub fn lex(line: &str) -> Result<Vec<Token>, LexError> {
    shell_lexer::line(line).map_err(|e| classify(&e))
}

/// Turns the sentinel string chosen at the failing rule's `{? Err(...) }` site back into the
/// `LexError` it stands for. Mechanical only: the actual decision about what went wrong was already
/// made, in the grammar, at the position where it happened.
fn classify<L>(e: &peg::error::ParseError<L>) -> LexError {
    for sentinel in e.expected.tokens() {
        match sentinel {
            "unterminated-quote" => return LexError::Unterminated("quote"),
            "unterminated-escape" => return LexError::Unterminated("escape"),
            "bad-substitution" => return LexError::BadSubstitution,
            "op-heredoc" => return LexError::Unsupported("<<"),
            "op-dupin" => return LexError::Unsupported("<&"),
            "op-semicolon" => return LexError::Unsupported(";"),
            "op-amp" => return LexError::Unsupported("&"),
            "op-lparen" => return LexError::Unsupported("("),
            "op-rparen" => return LexError::Unsupported(")"),
            _ => {}
        }
    }
    unreachable!("lexer grammar failure with no recognized sentinel in the expected set");
}

#[cfg(test)]
mod tests {
    use super::*;
    use RedirOp::*;

    fn w(text: &str) -> Token {
        Token::Word(Word::literal(text))
    }
    /// A word made of `parts`.
    fn parts(parts: Vec<Part>) -> Token {
        Token::Word(Word::from_parts(parts))
    }
    fn lit(text: &str) -> Part {
        Part::Lit(text.into())
    }
    fn var(name: &str, quoted: bool) -> Part {
        Part::Var { name: name.into(), quoted }
    }
    fn r(fd: Option<u8>, op: RedirOp) -> Token {
        Token::Redir { fd, op }
    }
    fn lexed(line: &str) -> Vec<Token> {
        lex(line).unwrap()
    }

    #[test]
    fn words_are_split_on_blanks() {
        assert_eq!(lexed("echo a b"), [w("echo"), w("a"), w("b")]);
        assert_eq!(lexed("  echo \t a   "), [w("echo"), w("a")]);
        assert_eq!(lexed(""), []);
        assert_eq!(lexed("   "), []);
    }

    #[test]
    fn quotes_group() {
        assert_eq!(lexed(r#"echo "a b""#), [w("echo"), w("a b")]);
        assert_eq!(lexed("echo 'a b'"), [w("echo"), w("a b")]);
        assert_eq!(lexed(r#"echo a"b c"d"#), [w("echo"), w("ab cd")]);
        assert_eq!(lexed(r#"echo """#), [w("echo"), w("")]); // an empty word
    }

    #[test]
    fn quoted_operators_are_text() {
        assert_eq!(lexed(r#"echo "|""#), [w("echo"), w("|")]);
        assert_eq!(lexed("echo '|'"), [w("echo"), w("|")]);
        assert_eq!(
            lexed(r#"echo ">" '<' ">>""#),
            [w("echo"), w(">"), w("<"), w(">>")]
        );
        assert_eq!(
            lexed(r#"echo ";" "&" "(""#),
            [w("echo"), w(";"), w("&"), w("(")]
        );
    }

    #[test]
    fn single_quotes_are_fully_literal() {
        assert_eq!(lexed(r"echo 'a\b'"), [w("echo"), w(r"a\b")]);
        assert_eq!(lexed(r#"echo 'a"b'"#), [w("echo"), w(r#"a"b"#)]);
        assert_eq!(lexed("echo '$x `y`'"), [w("echo"), w("$x `y`")]);
    }

    #[test]
    fn double_quotes_escape_only_four_characters() {
        assert_eq!(lexed(r#"echo "a\"b""#), [w("echo"), w(r#"a"b"#)]);
        assert_eq!(lexed(r#"echo "a\\b""#), [w("echo"), w(r"a\b")]);
        assert_eq!(lexed(r#"echo "\$""#), [w("echo"), w("$")]);
        assert_eq!(lexed(r#"echo "\`""#), [w("echo"), w("`")]);
        assert_eq!(lexed(r#"echo "a\nb""#), [w("echo"), w(r"a\nb")]); // any other backslash is itself
        assert_eq!(lexed(r#"echo "\ ""#), [w("echo"), w(r"\ ")]);
    }

    #[test]
    fn an_unquoted_backslash_escapes_the_next_character() {
        assert_eq!(lexed(r"echo a\ b"), [w("echo"), w("a b")]);
        assert_eq!(lexed(r"echo \|"), [w("echo"), w("|")]);
        assert_eq!(lexed(r"echo \>x"), [w("echo"), w(">x")]);
        assert_eq!(lexed(r"echo \\"), [w("echo"), w(r"\")]);
        assert_eq!(lexed(r"echo \#a"), [w("echo"), w("#a")]);
    }

    #[test]
    fn unterminated_quotes_and_escapes_are_errors() {
        assert_eq!(lex(r#"echo "abc"#), Err(LexError::Unterminated("quote")));
        assert_eq!(lex("echo 'abc"), Err(LexError::Unterminated("quote")));
        assert_eq!(lex(r"echo abc\"), Err(LexError::Unterminated("escape")));
        assert_eq!(lex(r#"echo "abc\"#), Err(LexError::Unterminated("quote")));
    }

    #[test]
    fn a_comment_starts_only_at_the_start_of_a_word() {
        assert_eq!(lexed("# nothing here"), []);
        assert_eq!(lexed("echo a #b c"), [w("echo"), w("a")]);
        assert_eq!(lexed("echo a#b"), [w("echo"), w("a#b")]);
        assert_eq!(lexed("echo #"), [w("echo")]);
        assert_eq!(lexed(r##"echo "#""##), [w("echo"), w("#")]);
        assert_eq!(
            lexed("echo a | # trailing"),
            [w("echo"), w("a"), Token::Pipe]
        );
    }

    #[test]
    fn pipes_and_redirections_end_words() {
        assert_eq!(lexed("a|b"), [w("a"), Token::Pipe, w("b")]);
        assert_eq!(lexed("a>b"), [w("a"), r(None, Out), w("b")]);
        assert_eq!(lexed("a>>b"), [w("a"), r(None, Append), w("b")]);
        assert_eq!(lexed("a<b"), [w("a"), r(None, In), w("b")]);
        assert_eq!(
            lexed("a | b > c < d"),
            [
                w("a"),
                Token::Pipe,
                w("b"),
                r(None, Out),
                w("c"),
                r(None, In),
                w("d")
            ]
        );
        assert_eq!(lexed("a >&2"), [w("a"), r(None, DupOut), w("2")]);
    }

    #[test]
    fn a_descriptor_number_right_before_an_operator_is_taken() {
        assert_eq!(lexed("cmd 2>x"), [w("cmd"), r(Some(2), Out), w("x")]);
        assert_eq!(lexed("cmd 1>x"), [w("cmd"), r(Some(1), Out), w("x")]);
        assert_eq!(lexed("cmd 2>>x"), [w("cmd"), r(Some(2), Append), w("x")]);
        assert_eq!(lexed("cmd 2>&1"), [w("cmd"), r(Some(2), DupOut), w("1")]);
        assert_eq!(lexed("cmd 0<x"), [w("cmd"), r(Some(0), In), w("x")]);
    }

    #[test]
    fn digits_elsewhere_are_ordinary_text() {
        assert_eq!(lexed("a2>x"), [w("a2"), r(None, Out), w("x")]);
        assert_eq!(
            lexed("echo 2 >x"),
            [w("echo"), w("2"), r(None, Out), w("x")]
        );
        assert_eq!(
            lexed(r#"echo "2">x"#),
            [w("echo"), w("2"), r(None, Out), w("x")]
        );
        assert_eq!(
            lexed(r"echo \2>x"),
            [w("echo"), w("2"), r(None, Out), w("x")]
        );
        assert_eq!(lexed("cmd 3>x"), [w("cmd"), w("3"), r(None, Out), w("x")]);
        assert_eq!(lexed("cmd 12>x"), [w("cmd"), w("12"), r(None, Out), w("x")]);
        assert_eq!(lexed("cmd 2<x"), [w("cmd"), w("2"), r(None, In), w("x")]); // 2 is not an input descriptor
        assert_eq!(lexed("cmd 0>x"), [w("cmd"), w("0"), r(None, Out), w("x")]); // nor 0 an output one
    }

    #[test]
    fn globbing_characters_and_backticks_are_ordinary() {
        assert_eq!(
            lexed("echo `y` * ? ~ {a,b}"),
            [w("echo"), w("`y`"), w("*"), w("?"), w("~"), w("{a,b}")]
        );
    }

    #[test]
    fn a_variable_is_a_part_of_its_word() {
        assert_eq!(lexed("echo $x"), [w("echo"), parts(vec![var("x", false)])]);
        assert_eq!(lexed("echo ${x}"), [w("echo"), parts(vec![var("x", false)])]);
        assert_eq!(lexed("echo $x_1y"), [w("echo"), parts(vec![var("x_1y", false)])]);
        assert_eq!(lexed("echo $_"), [w("echo"), parts(vec![var("_", false)])]);
        assert_eq!(lexed("echo $?"), [w("echo"), parts(vec![Part::Status { quoted: false }])]);
    }

    #[test]
    fn a_name_takes_the_longest_run_and_the_rest_is_text() {
        assert_eq!(lexed("echo $a-b"), [w("echo"), parts(vec![var("a", false), lit("-b")])]);
        assert_eq!(lexed("echo $a.txt"), [w("echo"), parts(vec![var("a", false), lit(".txt")])]);
        assert_eq!(lexed("echo x$a$b"), [w("echo"), parts(vec![lit("x"), var("a", false), var("b", false)])]);
        assert_eq!(lexed("echo ${a}b"), [w("echo"), parts(vec![var("a", false), lit("b")])]);
        assert_eq!(lexed("echo $ab"), [w("echo"), parts(vec![var("ab", false)])]);
        assert_eq!(lexed("echo $?x"), [w("echo"), parts(vec![Part::Status { quoted: false }, lit("x")])]);
    }

    #[test]
    fn a_dollar_that_names_nothing_is_text() {
        assert_eq!(lexed("echo $"), [w("echo"), w("$")]);
        assert_eq!(lexed("echo $ x"), [w("echo"), w("$"), w("x")]);
        assert_eq!(lexed("echo $1 $$ $@ $# $-"), [w("echo"), w("$1"), w("$$"), w("$@"), w("$#"), w("$-")]);
        assert_eq!(lexed("echo a$"), [w("echo"), w("a$")]);
        assert_eq!(lexed(r#"echo "$""#), [w("echo"), w("$")]);
        assert_eq!(lexed(r#"echo "a$ b""#), [w("echo"), w("a$ b")]);
    }

    #[test]
    fn quoting_decides_what_an_expansion_is() {
        assert_eq!(lexed(r#"echo "$x""#), [w("echo"), parts(vec![var("x", true)])]);
        assert_eq!(lexed(r#"echo "a $x b""#), [w("echo"), parts(vec![lit("a "), var("x", true), lit(" b")])]);
        assert_eq!(lexed(r#"echo "${x}y""#), [w("echo"), parts(vec![var("x", true), lit("y")])]);
        assert_eq!(lexed(r#"echo "$?""#), [w("echo"), parts(vec![Part::Status { quoted: true }])]);
        // Mixed in one word: each part keeps its own quoting.
        assert_eq!(lexed(r#"echo $a"$b""#), [w("echo"), parts(vec![var("a", false), var("b", true)])]);
        assert_eq!(lexed(r#"echo "$a"$b"#), [w("echo"), parts(vec![var("a", true), var("b", false)])]);
        // Literal: single quotes and a backslash, in either context.
        assert_eq!(lexed("echo '$x'"), [w("echo"), w("$x")]);
        assert_eq!(lexed(r"echo \$x"), [w("echo"), w("$x")]);
        assert_eq!(lexed(r#"echo "\$x""#), [w("echo"), w("$x")]);
        assert_eq!(lexed(r#"echo a'$x'"$y""#), [w("echo"), parts(vec![lit("a$x"), var("y", true)])]);
    }

    #[test]
    fn an_empty_quoted_word_stays_a_word() {
        assert_eq!(lexed(r#"echo """#), [w("echo"), w("")]);
        assert_eq!(lexed(r#"echo $x"""#), [w("echo"), parts(vec![var("x", false), lit("")])]);
    }

    #[test]
    fn a_variable_does_not_hide_an_operator() {
        assert_eq!(lexed("echo $x>f"), [w("echo"), parts(vec![var("x", false)]), r(None, Out), w("f")]);
        assert_eq!(lexed("echo $x|cat"), [w("echo"), parts(vec![var("x", false)]), Token::Pipe, w("cat")]);
    }

    fn assignment(line: &str) -> Option<(String, Word)> {
        let Token::Word(word) = &lexed(line)[0] else { panic!("not a word") };
        word.assignment().map(|(name, value)| (name.to_string(), value))
    }

    #[test]
    fn a_word_that_starts_with_name_equals_is_an_assignment() {
        assert_eq!(assignment("A=b"), Some(("A".into(), Word::literal("b"))));
        assert_eq!(assignment("A="), Some(("A".into(), Word::literal(""))));
        assert_eq!(assignment("_x1=a=b"), Some(("_x1".into(), Word::literal("a=b"))));
        assert_eq!(assignment("A=\"a b\"c"), Some(("A".into(), Word::literal("a bc"))));
        assert_eq!(assignment("A='$x'"), Some(("A".into(), Word::literal("$x"))));
        assert_eq!(assignment("A=\"\""), Some(("A".into(), Word::literal(""))));
        assert_eq!(
            assignment("A=$x"),
            Some(("A".into(), Word::from_parts(vec![var("x", false)])))
        );
        assert_eq!(
            assignment("A=pre${x}post"),
            Some(("A".into(), Word::from_parts(vec![lit("pre"), var("x", false), lit("post")])))
        );
        assert_eq!(assignment("A=$x"), assignment("A=$x"));
    }

    #[test]
    fn anything_else_is_an_ordinary_word() {
        assert_eq!(assignment("a"), None);
        assert_eq!(assignment("=x"), None); // no name
        assert_eq!(assignment("1A=x"), None); // not a valid name
        assert_eq!(assignment("a-b=x"), None);
        assert_eq!(assignment("a.b=x"), None);
        assert_eq!(assignment("\"A\"=x"), None); // the name is quoted...
        assert_eq!(assignment("A\"=\"x"), None); // ...or the `=` is
        assert_eq!(assignment("'A'=x"), None);
        assert_eq!(assignment(r"A\=x"), None);
        assert_eq!(assignment("$A=x"), None); // an expansion is not a name
        assert_eq!(assignment("--lines=5"), None);
    }

    #[test]
    fn an_assignment_is_still_the_word_as_typed() {
        assert_eq!(lexed("A=b")[0], Token::Word(Word { parts: vec![lit("A=b")], assign: Some(1) }));
        assert_eq!(lexed("A=b").len(), 1);
        assert_eq!(lexed("a=b c=d").len(), 2);
    }

    #[test]
    fn a_bad_substitution_is_an_error() {
        assert_eq!(lex("echo ${"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${}"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${1}"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${a-b}"), Err(LexError::BadSubstitution));
        assert_eq!(lex("echo ${a"), Err(LexError::BadSubstitution));
        assert_eq!(lex(r#"echo "${a b}""#), Err(LexError::BadSubstitution));
        assert_eq!(lex(r#"echo "${""#), Err(LexError::BadSubstitution));
    }

    #[test]
    fn unsupported_syntax_is_refused() {
        assert_eq!(lex("a; b"), Err(LexError::Unsupported(";")));
        assert_eq!(lex("a && b"), Err(LexError::Unsupported("&")));
        assert_eq!(lex("a &"), Err(LexError::Unsupported("&")));
        assert_eq!(lex("(a)"), Err(LexError::Unsupported("(")));
        assert_eq!(lex("a <<EOF"), Err(LexError::Unsupported("<<")));
        assert_eq!(lex("a <&0"), Err(LexError::Unsupported("<&")));
    }
}
