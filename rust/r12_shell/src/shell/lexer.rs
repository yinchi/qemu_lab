//! Splits a command line into tokens: words, `|`, and redirection operators. Replaces `shlex`, which
//! threw away whether a word was quoted -- so `echo "|"` would have looked like a pipe, and `a>b`
//! stayed one word.
//!
//! The rules are POSIX's, as far as this shell goes:
//! - A word is what lies between blanks. Single quotes are fully literal. Inside double quotes everything is
//!   literal except that a backslash escapes `"`, `\`, `$` and a backtick (so `"\$"` is `$`, which stays true
//!   when a later stage gives `$` a meaning); any other backslash there is itself. An unquoted backslash makes
//!   the next character literal. Quoting or escaping any part of a word makes the whole word `quoted`.
//! - `#` starts a comment only at the start of a word: `echo a#b` prints `a#b`, `echo a #b` prints `a`.
//! - `|`, `<`, `>`, `>>` and `>&` end a word and are operators. An unquoted, unescaped word that is just the
//!   digit `1` or `2` (or `0` before `<`), *immediately* followed by an operator, is that operator's file
//!   descriptor number (`2>err`, `2>>err`, `2>&1`); anywhere else digits are ordinary text (`a2>x` is the word
//!   `a2` and `>x`; `echo 2 >x` echoes `2`; a quoted `"2">x` is a word; `3>x` is the word `3` and `>x`).
//! - `$`, backtick, `*`, `?`, `~`, `{` and `}` are ordinary characters: nothing is expanded or globbed yet.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A word. `quoted` is whether any part of it was quoted or escaped.
    Word { text: String, quoted: bool },
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
        rule unquoted() -> String = s:$(unquoted_start() unquoted_tail_char()*) { s.to_string() }

        rule squote_char() = !"'" [_]
        rule squoted() -> String
            = "'" s:$(squote_char()*) "'" { s.to_string() }
            / "'" squote_char()* {? Err("unterminated-quote") }
        // No escaping inside single quotes.

        rule dquote_escape() -> char = "\\" c:['"' | '\\' | '$' | '`'] { c }
        rule dquote_char() -> char = dquote_escape() / (!"\"" c:[_] { c })
        // A backslash not immediately before one of the four specials falls through to the catch-all
        // and is kept, as itself, by the *next* dquote_char -- pushing a lone '\\' and letting the
        // following character (or end of input) be handled normally on the next iteration, rather
        // than needing a dedicated "backslash but not before a special" rule.
        rule dquoted() -> String
            = "\"" s:dquote_char()* "\"" { s.into_iter().collect() }
            / "\"" dquote_char()* {? Err("unterminated-quote") }

        rule escape_piece() -> String
            = "\\" c:[_] { c.to_string() }
            / "\\" {? Err("unterminated-escape") }
        // Outside any quote, a backslash escapes exactly the next character, whatever it is; with
        // nothing after it (end of input), that's an error distinct from an unterminated quote.

        rule piece() -> String = dquoted() / squoted() / escape_piece()
        rule tail() -> String
            = p:piece() s:$(unquoted_tail_char()*) { let mut r = p; r.push_str(s); r }

        // A word is unquoted text with zero or more (piece, then more text) pairs after it, or -- if
        // it starts with a piece instead -- one or more of those same pairs.
        rule word_token() -> (String, bool)
            = u:unquoted() ts:tail()* {
                let quoted = !ts.is_empty();
                let mut s = u;
                for t in ts { s.push_str(&t); }
                (s, quoted)
              }
            / ts:tail()+ {
                let mut s = String::new();
                for t in &ts { s.push_str(t); }
                (s, true)
              }

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
            / w:word_token() { Token::Word { text: w.0, quoted: w.1 } }

        pub rule line() -> Vec<Token>
            = ts:(t:token() {Some(t)} / blank() {None} / comment() {None})* {
                ts.into_iter().flatten().collect()
              }
    }
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
        Token::Word {
            text: text.into(),
            quoted: false,
        }
    }
    fn q(text: &str) -> Token {
        Token::Word {
            text: text.into(),
            quoted: true,
        }
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
    fn quotes_group_and_mark_the_word() {
        assert_eq!(lexed(r#"echo "a b""#), [w("echo"), q("a b")]);
        assert_eq!(lexed("echo 'a b'"), [w("echo"), q("a b")]);
        assert_eq!(lexed(r#"echo a"b c"d"#), [w("echo"), q("ab cd")]);
        assert_eq!(lexed(r#"echo """#), [w("echo"), q("")]); // an empty word
    }

    #[test]
    fn quoted_operators_are_text() {
        assert_eq!(lexed(r#"echo "|""#), [w("echo"), q("|")]);
        assert_eq!(lexed("echo '|'"), [w("echo"), q("|")]);
        assert_eq!(
            lexed(r#"echo ">" '<' ">>""#),
            [w("echo"), q(">"), q("<"), q(">>")]
        );
        assert_eq!(
            lexed(r#"echo ";" "&" "(""#),
            [w("echo"), q(";"), q("&"), q("(")]
        );
    }

    #[test]
    fn single_quotes_are_fully_literal() {
        assert_eq!(lexed(r"echo 'a\b'"), [w("echo"), q(r"a\b")]);
        assert_eq!(lexed(r#"echo 'a"b'"#), [w("echo"), q(r#"a"b"#)]);
        assert_eq!(lexed("echo '$x `y`'"), [w("echo"), q("$x `y`")]);
    }

    #[test]
    fn double_quotes_escape_only_four_characters() {
        assert_eq!(lexed(r#"echo "a\"b""#), [w("echo"), q(r#"a"b"#)]);
        assert_eq!(lexed(r#"echo "a\\b""#), [w("echo"), q(r"a\b")]);
        assert_eq!(lexed(r#"echo "\$""#), [w("echo"), q("$")]);
        assert_eq!(lexed(r#"echo "\`""#), [w("echo"), q("`")]);
        assert_eq!(lexed(r#"echo "a\nb""#), [w("echo"), q(r"a\nb")]); // any other backslash is itself
        assert_eq!(lexed(r#"echo "\ ""#), [w("echo"), q(r"\ ")]);
    }

    #[test]
    fn an_unquoted_backslash_escapes_the_next_character() {
        assert_eq!(lexed(r"echo a\ b"), [w("echo"), q("a b")]);
        assert_eq!(lexed(r"echo \|"), [w("echo"), q("|")]);
        assert_eq!(lexed(r"echo \>x"), [w("echo"), q(">x")]);
        assert_eq!(lexed(r"echo \\"), [w("echo"), q(r"\")]);
        assert_eq!(lexed(r"echo \#a"), [w("echo"), q("#a")]);
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
        assert_eq!(lexed(r##"echo "#""##), [w("echo"), q("#")]);
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
            [w("echo"), q("2"), r(None, Out), w("x")]
        );
        assert_eq!(
            lexed(r"echo \2>x"),
            [w("echo"), q("2"), r(None, Out), w("x")]
        );
        assert_eq!(lexed("cmd 3>x"), [w("cmd"), w("3"), r(None, Out), w("x")]);
        assert_eq!(lexed("cmd 12>x"), [w("cmd"), w("12"), r(None, Out), w("x")]);
        assert_eq!(lexed("cmd 2<x"), [w("cmd"), w("2"), r(None, In), w("x")]); // 2 is not an input descriptor
        assert_eq!(lexed("cmd 0>x"), [w("cmd"), w("0"), r(None, Out), w("x")]); // nor 0 an output one
    }

    #[test]
    fn expansion_characters_are_ordinary() {
        assert_eq!(
            lexed("echo $x `y` * ? ~ {a,b}"),
            [
                w("echo"),
                w("$x"),
                w("`y`"),
                w("*"),
                w("?"),
                w("~"),
                w("{a,b}")
            ]
        );
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
