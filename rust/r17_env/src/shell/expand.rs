//! Gives the words of a command their values, just before the command runs: `$NAME`, `${NAME}` and `$?` replaced
//! by what they stand for, and the result cut into fields where POSIX says to. Pure `no_std` + `alloc`, so it is
//! tested on the host (`hosttests/`); the variables come in through a lookup function, so nothing here knows
//! about frames.
//!
//! The rules, as in a POSIX shell:
//! - Literal text is never split. A quoted expansion (`"$X"`) is one piece of its word, whatever it holds,
//!   including nothing: `"$EMPTY"` is a word of no characters, an argument.
//! - An unquoted expansion's value is split into fields at runs of blanks (space, tab, newline). Leading and
//!   trailing blanks make no empty field, and a value of only blanks (or nothing) gives no field at all: an
//!   unquoted `$EMPTY` disappears from the command line. The pieces around it join the first and last field
//!   (`a$X` with `X="1 2"` is `a1` and `2`).
//! - A variable that is not set is the empty string. `$?` is the status the caller passes.
//! - An assignment's value (`A=$X`) is not split at all: `expand_assignment` joins the parts.

use alloc::string::String;
use alloc::vec::Vec;

use super::lexer::{Part, Word};

/// Where the values come from.
pub struct Values<'a> {
    /// The value of the variable `name`, or `None` if it is not set.
    pub lookup: &'a dyn Fn(&str) -> Option<String>,
    /// What `$?` stands for.
    pub status: i32,
}

fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

impl Values<'_> {
    /// The text an expansion part stands for. `None` for a literal.
    fn value(&self, part: &Part) -> Option<String> {
        match part {
            Part::Lit(_) => None,
            Part::Var { name, .. } => Some((self.lookup)(name).unwrap_or_default()),
            Part::Status { .. } => Some(alloc::format!("{}", self.status)),
        }
    }
}

/// The fields `word` becomes: none, one, or several (see the module comment).
pub fn expand_word(word: &Word, values: &Values) -> Vec<String> {
    let mut fields = Vec::new();
    // The field being built; `None` until something (even an empty quoted piece) has been added to it.
    let mut current: Option<String> = None;
    for part in &word.parts {
        let quoted = match part {
            Part::Lit(text) => {
                current.get_or_insert_with(String::new).push_str(text);
                continue;
            }
            Part::Var { quoted, .. } | Part::Status { quoted } => *quoted,
        };
        let value = values.value(part).unwrap_or_default();
        if quoted {
            current.get_or_insert_with(String::new).push_str(&value);
            continue;
        }
        // Unquoted: a run of blanks ends the field so far; other text extends it.
        let mut rest = value.as_str();
        while !rest.is_empty() {
            let blank = rest.starts_with(is_blank);
            let end = rest.find(|c: char| is_blank(c) != blank).unwrap_or(rest.len());
            let (run, after) = rest.split_at(end);
            if blank {
                fields.extend(current.take());
            } else {
                current.get_or_insert_with(String::new).push_str(run);
            }
            rest = after;
        }
    }
    fields.extend(current);
    fields
}

/// The arguments `words` become, in order.
pub fn expand(words: &[Word], values: &Values) -> Vec<String> {
    words.iter().flat_map(|word| expand_word(word, values)).collect()
}

/// The commands whose operands that look like assignments (`NAME=value`) are expanded as assignments are, not
/// split into fields: `export A=$X` sets `A` to all of `X`, as it does in bash.
const DECLARATION_COMMANDS: &[&str] = &["export"];

/// `expand`, for the words of a whole command: the same, except that after a declaration command
/// (`DECLARATION_COMMANDS`) an operand of the form `NAME=value` becomes the one field `NAME=` and the value
/// expanded like an assignment's (`expand_assignment`).
pub fn expand_command(words: &[Word], values: &Values) -> Vec<String> {
    let declaration = words
        .first()
        .and_then(Word::as_literal)
        .is_some_and(|name| DECLARATION_COMMANDS.contains(&name));
    if !declaration {
        return expand(words, values);
    }
    let mut fields = expand_word(&words[0], values);
    for word in &words[1..] {
        match word.assignment() {
            Some((name, value)) => {
                fields.push(alloc::format!("{name}={}", expand_assignment(&value, values)));
            }
            None => fields.extend(expand_word(word, values)),
        }
    }
    fields
}

/// An assignment's value: the parts joined, with no field splitting and so no vanishing (`A=$EMPTY` is empty,
/// `A=$X` keeps every blank of `X`), whatever the quoting.
pub fn expand_assignment(word: &Word, values: &Values) -> String {
    let mut text = String::new();
    for part in &word.parts {
        match part {
            Part::Lit(lit) => text.push_str(lit),
            other => text.push_str(&values.value(other).unwrap_or_default()),
        }
    }
    text
}

/// A redirection's file name: `Err` unless `word` becomes exactly one field (bash's `ambiguous redirect`).
pub fn expand_target(word: &Word, values: &Values) -> Result<String, Ambiguous> {
    let mut fields = expand_word(word, values);
    match fields.len() {
        1 => Ok(fields.remove(0)),
        _ => Err(Ambiguous),
    }
}

/// A word that was to name one file became none or several.
#[derive(Debug, PartialEq, Eq)]
pub struct Ambiguous;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{Token, lex};

    fn words(line: &str) -> Vec<Word> {
        lex(line)
            .unwrap()
            .into_iter()
            .map(|t| match t {
                Token::Word(word) => word,
                other => panic!("not a word: {other:?}"),
            })
            .collect()
    }

    /// Expands `line`'s words with `X="1 2"`, `E=""`, `B="  "`, `S=" a b "`, `N=nospace`, `P="a*b"` set, `status` 7.
    fn ex(line: &str) -> Vec<String> {
        let lookup = |name: &str| {
            let v = match name {
                "X" => "1 2",
                "E" => "",
                "B" => "  \t ",
                "S" => " a  b ",
                "N" => "nospace",
                "P" => "a*b",
                "L" => "l1\nl2",
                _ => return None,
            };
            Some(String::from(v))
        };
        expand(&words(line), &Values { lookup: &lookup, status: 7 })
    }

    #[test]
    fn plain_words_are_unchanged() {
        assert_eq!(ex("a b  c"), ["a", "b", "c"]);
        assert_eq!(ex("a 'b c' \"d e\""), ["a", "b c", "d e"]);
        assert_eq!(ex(""), Vec::<String>::new());
    }

    #[test]
    fn a_variable_is_replaced_by_its_value() {
        assert_eq!(ex("echo $N"), ["echo", "nospace"]);
        assert_eq!(ex("echo ${N}"), ["echo", "nospace"]);
        assert_eq!(ex("echo pre${N}post"), ["echo", "prenospacepost"]);
        assert_eq!(ex("echo $N$N"), ["echo", "nospacenospace"]);
        assert_eq!(ex("echo $N.txt"), ["echo", "nospace.txt"]);
        assert_eq!(ex("echo ${N}txt"), ["echo", "nospacetxt"]);
        assert_eq!(ex("echo $Nx"), ["echo"]); // `Nx` is the name, and it is not set
    }

    #[test]
    fn an_unset_variable_is_empty() {
        assert_eq!(ex("echo $NOPE"), ["echo"]);
        assert_eq!(ex(r#"echo "$NOPE""#), ["echo", ""]);
        assert_eq!(ex("echo a${NOPE}b"), ["echo", "ab"]);
    }

    #[test]
    fn an_unquoted_value_splits_at_blanks() {
        assert_eq!(ex("echo $X"), ["echo", "1", "2"]);
        assert_eq!(ex("echo $S"), ["echo", "a", "b"]); // no empty fields from the edges or the double blank
        assert_eq!(ex("echo $L"), ["echo", "l1", "l2"]); // a newline is a blank too
        assert_eq!(ex("echo $X$X"), ["echo", "1", "21", "2"]); // the last field of one joins the first of the next
    }

    #[test]
    fn a_value_of_only_blanks_or_nothing_is_no_field() {
        assert_eq!(ex("echo $E"), ["echo"]);
        assert_eq!(ex("echo $B"), ["echo"]);
        assert_eq!(ex("echo $E$B$E"), ["echo"]);
        assert_eq!(ex("echo x$B"), ["echo", "x"]);
        assert_eq!(ex("echo $Bx"), ["echo"]); // `Bx` is another name, not set
        assert_eq!(ex("echo ${B}x"), ["echo", "x"]);
    }

    #[test]
    fn text_around_an_expansion_joins_the_first_and_last_field() {
        assert_eq!(ex("echo a$X"), ["echo", "a1", "2"]);
        assert_eq!(ex("echo $X-z"), ["echo", "1", "2-z"]);
        assert_eq!(ex("echo a${X}z"), ["echo", "a1", "2z"]);
        assert_eq!(ex("echo a${S}z"), ["echo", "a", "a", "b", "z"]);
    }

    #[test]
    fn a_quoted_value_is_one_field_however_it_looks() {
        assert_eq!(ex(r#"echo "$X""#), ["echo", "1 2"]);
        assert_eq!(ex(r#"echo "$S""#), ["echo", " a  b "]);
        assert_eq!(ex(r#"echo "$B""#), ["echo", "  \t "]);
        assert_eq!(ex(r#"echo "$E""#), ["echo", ""]);
        assert_eq!(ex(r#"echo "a $X b""#), ["echo", "a 1 2 b"]);
        assert_eq!(ex(r#"echo "$L""#), ["echo", "l1\nl2"]);
    }

    #[test]
    fn quoting_the_empty_string_keeps_a_field() {
        assert_eq!(ex(r#"echo "" x"#), ["echo", "", "x"]);
        assert_eq!(ex(r#"echo $E"""#), ["echo", ""]);
        assert_eq!(ex(r#"echo ''$E"#), ["echo", ""]);
    }

    #[test]
    fn a_quoted_and_an_unquoted_part_of_one_word() {
        assert_eq!(ex(r#"echo "$X"$X"#), ["echo", "1 21", "2"]);
        assert_eq!(ex(r#"echo $X"$X""#), ["echo", "1", "21 2"]);
    }

    #[test]
    fn single_quotes_and_backslashes_stop_an_expansion() {
        assert_eq!(ex("echo '$X' \\$X \"\\$X\""), ["echo", "$X", "$X", "$X"]);
        assert_eq!(ex("echo a'$X'b"), ["echo", "a$Xb"]);
    }

    #[test]
    fn the_status_is_a_number() {
        assert_eq!(ex("echo $?"), ["echo", "7"]);
        assert_eq!(ex(r#"echo "$?" x$?y"#), ["echo", "7", "x7y"]);
        assert_eq!(ex("echo ${N}$?"), ["echo", "nospace7"]);
    }

    #[test]
    fn a_dollar_that_names_nothing_stays() {
        assert_eq!(ex("echo $ $1 a$"), ["echo", "$", "$1", "a$"]);
    }

    #[test]
    fn nothing_else_is_touched() {
        // No globbing, and the value is not looked at for further `$` or quotes.
        assert_eq!(ex("echo $P"), ["echo", "a*b"]);
        let lookup = |_: &str| Some(String::from("$N 'q'"));
        let got = expand(&words("echo $A"), &Values { lookup: &lookup, status: 0 });
        assert_eq!(got, ["echo", "$N", "'q'"]);
    }

    #[test]
    fn an_assignment_value_is_never_split() {
        let lookup = |name: &str| match name {
            "S" => Some(String::from(" a  b ")),
            "E" => Some(String::new()),
            _ => None,
        };
        let values = Values { lookup: &lookup, status: 3 };
        let value = |line: &str| {
            let word = words(line).remove(0);
            let (_, value) = word.assignment().unwrap();
            expand_assignment(&value, &values)
        };
        assert_eq!(value("A=$S"), " a  b ");
        assert_eq!(value("A=\"$S\""), " a  b ");
        assert_eq!(value("A=x${S}y"), "x a  b y");
        assert_eq!(value("A=$E"), "");
        assert_eq!(value("A=$NOSUCH"), "");
        assert_eq!(value("A="), "");
        assert_eq!(value("A=$?"), "3");
        assert_eq!(value("A='$S'"), "$S");
        assert_eq!(value("A=b=c"), "b=c");
    }

    #[test]
    fn export_operands_that_are_assignments_are_not_split() {
        let lookup = |name: &str| match name {
            "S" => Some(String::from(" a  b ")),
            "E" => Some(String::new()),
            _ => None,
        };
        let values = Values { lookup: &lookup, status: 0 };
        let cmd = |line: &str| expand_command(&words(line), &values);
        assert_eq!(cmd("export A=$S"), ["export", "A= a  b "]);
        assert_eq!(cmd("export A=$S B=$E C"), ["export", "A= a  b ", "B=", "C"]);
        assert_eq!(cmd("export A=x$S$S"), ["export", "A=x a  b  a  b "]);
        // An operand that is not an assignment is split as any other is.
        assert_eq!(cmd("export $S"), ["export", "a", "b"]);
        assert_eq!(cmd("export $S=1"), ["export", "a", "b", "=1"]); // `$S=1` does not start with a name
        assert_eq!(cmd("export a-b=$S"), ["export", "a-b=", "a", "b"]);
        // Other commands are not declarations.
        assert_eq!(cmd("echo A=$S"), ["echo", "A=", "a", "b"]);
    }

    #[test]
    fn a_redirect_target_must_be_exactly_one_field() {
        let lookup = |name: &str| (name == "X").then(|| String::from("1 2"));
        let values = Values { lookup: &lookup, status: 0 };
        let one = |line: &str| expand_target(&words(line)[0], &values);
        assert_eq!(one("file"), Ok(String::from("file")));
        assert_eq!(one("$NOPE"), Err(Ambiguous));
        assert_eq!(one("$X"), Err(Ambiguous));
        assert_eq!(one(r#""$X""#), Ok(String::from("1 2")));
        assert_eq!(one(r#""$NOPE""#), Ok(String::new()));
        assert_eq!(one("out$X"), Err(Ambiguous));
    }
}
