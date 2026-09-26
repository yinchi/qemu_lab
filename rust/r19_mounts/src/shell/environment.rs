//! The initial environment: the parser for `/etc/environment`, the file Linux's `pam_env` reads. Plain `NAME=VALUE`
//! lines -- not shell syntax:
//!
//! - a blank line, or one whose first non-blank character is `#`, is ignored;
//! - otherwise the line is `NAME=VALUE`: the name is what precedes the first `=` (surrounding blanks trimmed) and
//!   must be a valid variable name; the value is **everything after that `=`, literally** (no quotes, no `$`
//!   expansion, blanks kept) except a trailing carriage return;
//! - a line that is not of that shape is skipped and reported, not fatal;
//! - a name that appears twice takes its last value, in the place it first appeared.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`hosttests/`). Reading the file and applying it to the shell's
//! frame is `shell::load_environment`.

use alloc::string::String;
use alloc::vec::Vec;

use crate::exec::frame_stack::is_valid_name; // in `hosttests`, `crate::exec` is an alias (its `lib.rs`)

/// A line that was skipped, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    /// 1-based.
    pub line: usize,
    pub why: &'static str,
}

/// What `parse` found: the variables in order, and the lines it had to skip.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub vars: Vec<(String, String)>,
    pub problems: Vec<Problem>,
}

/// Parses the text of an environment file.
pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let problem = |why| Problem { line: index + 1, why };
        let Some((name, value)) = line.split_once('=') else {
            parsed.problems.push(problem("expected NAME=VALUE (no '=')"));
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            parsed.problems.push(problem("expected NAME=VALUE (empty name)"));
            continue;
        }
        if !is_valid_name(name) {
            parsed.problems.push(problem("not a valid variable name"));
            continue;
        }
        match parsed.vars.iter_mut().find(|(n, _)| n == name) {
            Some((_, v)) => *v = String::from(value),
            None => parsed.vars.push((String::from(name), String::from(value))),
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(parsed: &Parsed) -> Vec<(&str, &str)> {
        parsed.vars.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect()
    }

    #[test]
    fn plain_lines_become_variables_in_order() {
        let p = parse("HOME=/root\nTZ=America/Toronto\n");
        assert_eq!(vars(&p), [("HOME", "/root"), ("TZ", "America/Toronto")]);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let p = parse("# the system environment\n\n   \n  # indented comment\nA=1\n");
        assert_eq!(vars(&p), [("A", "1")]);
        assert!(p.problems.is_empty());
    }

    #[test]
    fn the_value_is_literal_after_the_first_equals() {
        let p = parse("A=b=c\nB=\nC=  spaced  \nD=\"quoted\"\nE=$HOME\nF='x'\n");
        assert_eq!(
            vars(&p),
            [("A", "b=c"), ("B", ""), ("C", "  spaced  "), ("D", "\"quoted\""), ("E", "$HOME"), ("F", "'x'")]
        );
    }

    #[test]
    fn blanks_around_the_name_are_trimmed_but_not_inside_it() {
        let p = parse("  A  =1\nB C=2\n");
        assert_eq!(vars(&p), [("A", "1")]);
        assert_eq!(p.problems, [Problem { line: 2, why: "not a valid variable name" }]);
    }

    #[test]
    fn crlf_line_endings_and_a_missing_final_newline_are_fine() {
        let p = parse("A=1\r\nB=2");
        assert_eq!(vars(&p), [("A", "1"), ("B", "2")]);
    }

    #[test]
    fn a_bad_line_is_skipped_and_reported_with_its_number() {
        let p = parse("A=1\nnot a variable\n=orphan\n1x=2\nB=2\n");
        assert_eq!(vars(&p), [("A", "1"), ("B", "2")]);
        assert_eq!(
            p.problems,
            [
                Problem { line: 2, why: "expected NAME=VALUE (no '=')" },
                Problem { line: 3, why: "expected NAME=VALUE (empty name)" },
                Problem { line: 4, why: "not a valid variable name" },
            ]
        );
    }

    #[test]
    fn a_repeated_name_takes_its_last_value_in_its_first_place() {
        let p = parse("A=1\nB=2\nA=3\n");
        assert_eq!(vars(&p), [("A", "3"), ("B", "2")]);
    }

    #[test]
    fn an_empty_file_is_an_empty_environment() {
        assert_eq!(parse(""), Parsed::default());
        assert_eq!(parse("\n\n"), Parsed::default());
    }
}
