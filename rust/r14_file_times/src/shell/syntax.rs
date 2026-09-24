//! Turns a command line into a `Pipeline`: the stages joined by `|`, each with its words (the program
//! and its arguments) and its redirections in the order typed -- the order matters, since redirections
//! apply left to right, so `cmd > f 2>&1` and `cmd 2>&1 > f` differ.
//!
//! Only the shape is checked here: what a redirection's target names, and whether a program exists,
//! is for whoever runs it. Syntax errors are reported, not guessed at (`>` with no file name, `2>&` with
//! anything but `1` or `2`, an empty stage).
//!
//! Built on `peg`, over `lexer::lex`'s token stream rather than the raw line -- this is
//! `rust/docs/shell.ebnf`'s `segment`/`pipeline` productions directly. It runs over a token slice, not
//! `&str`; `peg`'s built-in slice support requires `Copy` elements (`peg-runtime`'s `ParseElem` trait
//! bounds `Element: Copy`), and `Token::Word` holds an owned `String`, so the grammar runs over
//! `SynTok` -- the same shape as `Token`, but a word carries its index into the original `&[Token]`
//! instead of the text itself -- and rules that need the actual text take `src: &[Token]` as an
//! ordinary rule parameter (`peg` rule parameters aren't implicitly shared across a grammar; each rule
//! on the path to one that needs `src` has to declare and pass it on). As in `lexer.rs`, every
//! `{? Err("...") }` block sits at the exact alternative that detects one specific problem (a
//! redirection with nothing after it, `>&` followed by neither `1` nor `2`, an empty stage), and
//! `classify` below is a flat conversion of the sentinel it chose back to a `SyntaxError`, not a
//! second pass over the tokens trying to work out what went wrong.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use super::lexer::{LexError, RedirOp, Token, lex};

/// `Token`, but with a word's text replaced by its index into the `&[Token]` this was built from --
/// `Copy`, so `peg`'s built-in `for [T]` support applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SynTok {
    Word(usize),
    Pipe,
    Redir { fd: Option<u8>, op: RedirOp },
}

fn to_syn_toks(tokens: &[Token]) -> Vec<SynTok> {
    tokens
        .iter()
        .enumerate()
        .map(|(i, t)| match t {
            Token::Word { .. } => SynTok::Word(i),
            Token::Pipe => SynTok::Pipe,
            Token::Redir { fd, op } => SynTok::Redir { fd: *fd, op: *op },
        })
        .collect()
}

/// The text of the word `src[i]` names. `to_syn_toks` only ever builds a `SynTok::Word(i)` from an
/// actual `Token::Word` at that index, so the other arms can't happen.
fn word_text(src: &[Token], i: usize) -> &str {
    match &src[i] {
        Token::Word { text, .. } => text.as_str(),
        _ => unreachable!("SynTok::Word index always points at a Token::Word"),
    }
}

/// One redirection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Redirection {
    /// `< path`: standard input from a file.
    In(String),
    /// `> path` / `2> path` (`append: false`), `>> path` / `2>> path` (`append: true`): a file for
    /// descriptor `fd` (1 or 2).
    Out { fd: u8, path: String, append: bool },
    /// `>&2`, `2>&1`: make `fd` (1 or 2) another name for what `target` (1 or 2) is.
    Dup { fd: u8, target: u8 },
}

/// One stage of a pipeline (`segment` in `Stage12.md`'s grammar, `rust/docs/shell.ebnf`): the program
/// (a builtin or a file) and its arguments, plus its own redirections, in the order typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Empty for a stage with only redirections (`> f` is valid: it creates the file, POSIX-style).
    pub argv: Vec<String>,
    pub redirs: Vec<Redirection>,
}

/// One or more `Segment`s joined by `|` -- exactly the grammar's `pipeline ::= segment ("|" segment)*`,
/// so a bare alias rather than a wrapping struct.
pub type Pipeline = Vec<Segment>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyntaxError {
    Lex(LexError),
    /// A redirection operator with no file name after it.
    MissingTarget(&'static str),
    /// `>&` followed by anything but `1` or `2`.
    BadDupTarget,
    /// A stage with no words and no redirections: `| a`, `a |`, `a | | b`.
    EmptyCommand,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntaxError::Lex(LexError::Unterminated("escape")) => {
                write!(f, "a backslash at the end of the line")
            }
            SyntaxError::Lex(LexError::Unterminated(what)) => write!(f, "unterminated {what}"),
            SyntaxError::Lex(LexError::Unsupported(what)) => write!(f, "`{what}` is not supported"),
            SyntaxError::MissingTarget(op) => write!(f, "no file name after `{op}`"),
            SyntaxError::BadDupTarget => write!(f, "`>&` must be followed by 1 or 2"),
            SyntaxError::EmptyCommand => write!(f, "missing command"),
        }
    }
}

/// A segment being built: either an argument word or a redirection, before being sorted into
/// `Segment`'s two separate `Vec`s.
enum Item {
    Arg(String),
    Redir(Redirection),
}

peg::parser! {
    grammar shell_syntax() for [SynTok] {
        rule word(src: &[Token]) -> String
            = [SynTok::Word(i)] { word_text(src, i).to_string() }

        // Each redirection kind gets two alternatives: the real one (operator, then its operand word)
        // and a fallback that matches the same operator alone and hard-fails. The fallback is only
        // ever reached once the first alternative has failed to find a following word -- and nothing
        // else in `item()` can match a bare redirection token either, so this is the furthest the
        // parse can go, exactly where `MissingTarget` belongs.
        rule redir(src: &[Token]) -> Redirection
            = [SynTok::Redir{fd, op: RedirOp::In}] path:word(src) { Redirection::In(path) }
            / [SynTok::Redir{op: RedirOp::In, ..}] {? Err("missing-target-in") }
            / [SynTok::Redir{fd, op: RedirOp::Out}] path:word(src) {
                Redirection::Out { fd: fd.unwrap_or(1), path, append: false }
              }
            / [SynTok::Redir{op: RedirOp::Out, ..}] {? Err("missing-target-out") }
            / [SynTok::Redir{fd, op: RedirOp::Append}] path:word(src) {
                Redirection::Out { fd: fd.unwrap_or(1), path, append: true }
              }
            / [SynTok::Redir{op: RedirOp::Append, ..}] {? Err("missing-target-append") }
            / [SynTok::Redir{fd, op: RedirOp::DupOut}] path:word(src) {?
                match path.as_str() {
                    "1" => Ok(Redirection::Dup { fd: fd.unwrap_or(1), target: 1 }),
                    "2" => Ok(Redirection::Dup { fd: fd.unwrap_or(1), target: 2 }),
                    _ => Err("bad-dup-target"),
                }
              }
            / [SynTok::Redir{op: RedirOp::DupOut, ..}] {? Err("missing-target-dup") }

        rule item(src: &[Token]) -> Item
            = w:word(src) { Item::Arg(w) }
            / r:redir(src) { Item::Redir(r) }

        // A segment's words and redirections interleave in any order/count; empty (no words, no
        // redirections -- a bare "|" on either side, or two in a row) is refused here, at the exact
        // point the real code checks it, not by the caller after the fact.
        rule segment(src: &[Token]) -> Segment
            = items:item(src)* {?
                let mut argv = Vec::new();
                let mut redirs = Vec::new();
                for it in items {
                    match it {
                        Item::Arg(a) => argv.push(a),
                        Item::Redir(r) => redirs.push(r),
                    }
                }
                if argv.is_empty() && redirs.is_empty() {
                    Err("empty-command")
                } else {
                    Ok(Segment { argv, redirs })
                }
              }

        pub rule pipeline(src: &[Token]) -> Vec<Segment>
            = s0:segment(src) rest:([SynTok::Pipe] s:segment(src) { s })* {
                let mut v = Vec::with_capacity(1 + rest.len());
                v.push(s0);
                v.extend(rest);
                v
              }
    }
}

/// Parses `line`. `Ok(None)` for a line with nothing on it (blank, or only a comment).
pub fn parse(line: &str) -> Result<Option<Pipeline>, SyntaxError> {
    let tokens = lex(line).map_err(SyntaxError::Lex)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let syn_toks = to_syn_toks(&tokens);
    shell_syntax::pipeline(&syn_toks, &tokens)
        .map(Some)
        .map_err(|e| classify(&e))
}

/// Turns the sentinel string chosen at the failing rule's `{? Err(...) }` site back into the
/// `SyntaxError` it stands for -- the same mechanical, no-guessing conversion as `lexer::classify`.
fn classify<L>(e: &peg::error::ParseError<L>) -> SyntaxError {
    for sentinel in e.expected.tokens() {
        match sentinel {
            "empty-command" => return SyntaxError::EmptyCommand,
            "bad-dup-target" => return SyntaxError::BadDupTarget,
            "missing-target-in" => return SyntaxError::MissingTarget(RedirOp::In.text()),
            "missing-target-out" => return SyntaxError::MissingTarget(RedirOp::Out.text()),
            "missing-target-append" => return SyntaxError::MissingTarget(RedirOp::Append.text()),
            "missing-target-dup" => return SyntaxError::MissingTarget(RedirOp::DupOut.text()),
            _ => {}
        }
    }
    unreachable!("syntax grammar failure with no recognized sentinel in the expected set");
}

#[cfg(test)]
mod tests {
    use super::*;
    use Redirection::*;

    fn seg(words: &[&str], redirs: Vec<Redirection>) -> Segment {
        Segment {
            argv: words.iter().map(|w| w.to_string()).collect(),
            redirs,
        }
    }
    fn one(line: &str) -> Segment {
        let p = parse(line).unwrap().unwrap();
        assert_eq!(p.len(), 1, "{line}");
        p.into_iter().next().unwrap()
    }

    #[test]
    fn a_blank_or_comment_line_is_nothing() {
        assert_eq!(parse(""), Ok(None));
        assert_eq!(parse("   \t "), Ok(None));
        assert_eq!(parse("# just a comment"), Ok(None));
    }

    #[test]
    fn a_plain_command() {
        assert_eq!(one("echo a b"), seg(&["echo", "a", "b"], vec![]));
        assert_eq!(one(r#"echo "a b" 'c'"#), seg(&["echo", "a b", "c"], vec![]));
        assert_eq!(one("echo a #b"), seg(&["echo", "a"], vec![]));
    }

    #[test]
    fn a_quoted_operator_is_an_argument_not_syntax() {
        assert_eq!(one(r#"echo "|""#), seg(&["echo", "|"], vec![]));
        assert_eq!(one("echo '>'"), seg(&["echo", ">"], vec![]));
        assert_eq!(one(r"echo a\ b"), seg(&["echo", "a b"], vec![]));
    }

    #[test]
    fn a_pipeline_has_a_stage_per_command() {
        let p = parse("a x | b | c > out < in").unwrap().unwrap();
        assert_eq!(
            p,
            vec![
                seg(&["a", "x"], vec![]),
                seg(&["b"], vec![]),
                seg(
                    &["c"],
                    vec![
                        Out {
                            fd: 1,
                            path: "out".into(),
                            append: false
                        },
                        In("in".into())
                    ]
                ),
            ]
        );
    }

    #[test]
    fn redirections_keep_the_order_typed() {
        assert_eq!(
            one("cmd > f 2>&1").redirs,
            vec![
                Out {
                    fd: 1,
                    path: "f".into(),
                    append: false
                },
                Dup { fd: 2, target: 1 }
            ]
        );
        assert_eq!(
            one("cmd 2>&1 > f").redirs,
            vec![
                Dup { fd: 2, target: 1 },
                Out {
                    fd: 1,
                    path: "f".into(),
                    append: false
                }
            ]
        );
    }

    #[test]
    fn output_redirections_default_to_stdout() {
        assert_eq!(
            one("cmd >f").redirs,
            vec![Out {
                fd: 1,
                path: "f".into(),
                append: false
            }]
        );
        assert_eq!(
            one("cmd >>f").redirs,
            vec![Out {
                fd: 1,
                path: "f".into(),
                append: true
            }]
        );
        assert_eq!(
            one("cmd 2>f").redirs,
            vec![Out {
                fd: 2,
                path: "f".into(),
                append: false
            }]
        );
        assert_eq!(
            one("cmd 2>>f").redirs,
            vec![Out {
                fd: 2,
                path: "f".into(),
                append: true
            }]
        );
        assert_eq!(
            one("cmd 1>f").redirs,
            vec![Out {
                fd: 1,
                path: "f".into(),
                append: false
            }]
        );
        assert_eq!(one("cmd <f").redirs, vec![In("f".into())]);
        assert_eq!(one("cmd 0<f").redirs, vec![In("f".into())]);
    }

    #[test]
    fn duplication_takes_one_or_two() {
        assert_eq!(one("cmd 2>&1").redirs, vec![Dup { fd: 2, target: 1 }]);
        assert_eq!(one("cmd >&2").redirs, vec![Dup { fd: 1, target: 2 }]);
        assert_eq!(one("cmd 1>&2").redirs, vec![Dup { fd: 1, target: 2 }]);
        assert_eq!(parse("cmd 2>&"), Err(SyntaxError::MissingTarget(">&")));
        assert_eq!(parse("cmd 2>&x"), Err(SyntaxError::BadDupTarget));
        assert_eq!(parse("cmd 2>&3"), Err(SyntaxError::BadDupTarget));
        assert_eq!(parse("cmd 2>&0"), Err(SyntaxError::BadDupTarget));
    }

    #[test]
    fn a_descriptor_number_is_a_redirection_only_where_the_lexer_says() {
        assert_eq!(
            one("a2>x"),
            seg(
                &["a2"],
                vec![Out {
                    fd: 1,
                    path: "x".into(),
                    append: false
                }]
            )
        );
        assert_eq!(
            one("echo 2 >x"),
            seg(
                &["echo", "2"],
                vec![Out {
                    fd: 1,
                    path: "x".into(),
                    append: false
                }]
            )
        );
        assert_eq!(
            one(r#"echo "2">x"#),
            seg(
                &["echo", "2"],
                vec![Out {
                    fd: 1,
                    path: "x".into(),
                    append: false
                }]
            )
        );
        assert_eq!(
            one("cmd 3>x"),
            seg(
                &["cmd", "3"],
                vec![Out {
                    fd: 1,
                    path: "x".into(),
                    append: false
                }]
            )
        );
    }

    #[test]
    fn a_missing_file_name_is_an_error() {
        assert_eq!(parse("cmd >"), Err(SyntaxError::MissingTarget(">")));
        assert_eq!(parse("cmd >>"), Err(SyntaxError::MissingTarget(">>")));
        assert_eq!(parse("cmd <"), Err(SyntaxError::MissingTarget("<")));
        assert_eq!(parse("cmd > | x"), Err(SyntaxError::MissingTarget(">")));
        assert_eq!(parse("cmd > > f"), Err(SyntaxError::MissingTarget(">")));
        assert_eq!(parse("cat >"), Err(SyntaxError::MissingTarget(">")));
    }

    #[test]
    fn an_empty_stage_is_an_error() {
        assert_eq!(parse("| a"), Err(SyntaxError::EmptyCommand));
        assert_eq!(parse("a |"), Err(SyntaxError::EmptyCommand));
        assert_eq!(parse("a | | b"), Err(SyntaxError::EmptyCommand));
        assert_eq!(parse("|"), Err(SyntaxError::EmptyCommand));
    }

    #[test]
    fn a_stage_of_only_redirections_parses() {
        // POSIX allows `> f` (it creates the file); running it is another matter.
        let c = one("> f");
        assert!(c.argv.is_empty());
        assert_eq!(
            c.redirs,
            vec![Out {
                fd: 1,
                path: "f".into(),
                append: false
            }]
        );
    }

    #[test]
    fn lexer_errors_pass_through() {
        assert_eq!(
            parse(r#"echo "x"#),
            Err(SyntaxError::Lex(LexError::Unterminated("quote")))
        );
        assert_eq!(
            parse("a; b"),
            Err(SyntaxError::Lex(LexError::Unsupported(";")))
        );
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            parse("echo 'x").unwrap_err().to_string(),
            "unterminated quote"
        );
        assert_eq!(
            parse("echo x\\").unwrap_err().to_string(),
            "a backslash at the end of the line"
        );
        assert_eq!(
            parse("a; b").unwrap_err().to_string(),
            "`;` is not supported"
        );
        assert_eq!(
            parse("cat >").unwrap_err().to_string(),
            "no file name after `>`"
        );
        assert_eq!(
            parse("a 2>&x").unwrap_err().to_string(),
            "`>&` must be followed by 1 or 2"
        );
        assert_eq!(parse("| a").unwrap_err().to_string(), "missing command");
    }
}
