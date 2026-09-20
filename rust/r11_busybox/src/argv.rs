//! Splits a finished line into a program name + arguments -- POSIX shell-word syntax (quotes,
//! backslash escapes), not just whitespace, since Stage 12's full shell reuses/extends this same
//! tokenizer once it needs to (see `ROADMAP.md`'s Stage 10 section), rather than this stage
//! inventing a narrower split now and rewriting it there.
//!
//! Built on `shlex`.

use alloc::string::String;
use alloc::vec::Vec;

/// A parsed command line, with the program name in argv[0].
pub struct Argv {
    words: Vec<String>,
}

/// Enumeration of possible errors when parsing a line into an `Argv`.
#[derive(Debug)]
pub enum ParseError {
    /// The line was empty, or only whitespace -- not an error a user should see; pressing Enter
    /// on a blank prompt just gets a new prompt, same as any real shell.
    Empty,
    /// `shlex::split` rejected the line as malformed shell syntax (e.g. an unterminated quote).
    Malformed,
}

impl Argv {
    /// Parses `line` into a program name + arguments.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let words = shlex::split(line).ok_or(ParseError::Malformed)?;
        if words.is_empty() {
            return Err(ParseError::Empty);
        }
        Ok(Self { words })
    }

    /// The program name (`argv[0]`).
    pub fn program(&self) -> &str {
        &self.words[0]
    }

    /// The full `argv` array, `program()` included -- what `process::run_program` wants to
    /// write onto the launched program's own stack.
    pub fn as_argv(&self) -> alloc::vec::Vec<&str> {
        self.words.iter().map(String::as_str).collect()
    }
}
