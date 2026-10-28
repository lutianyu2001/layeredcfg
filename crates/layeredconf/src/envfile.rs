//! Env-style files: `NAME=value` lines, as python-dotenv reads them for the most part.

use std::collections::BTreeMap;

use crate::text;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    /// Where the statement is in the text: from the start of its first line to past the end of its last (a quoted
    /// value may go on over several lines).
    pub begin: usize,
    pub end: usize,
}

/// A variable's value by its name, for `${NAME}` (the environment).
pub type Lookup<'a> = &'a dyn Fn(&[u8]) -> Option<Vec<u8>>;

fn is_newline(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

/// Where `needle` first is in `hay` from `from` on.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|window| window == needle).map(|at| at + from)
}

struct Reader<'a> {
    s: &'a [u8],
    lookup: Lookup<'a>,
    at: usize,
    known: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl<'a> Reader<'a> {
    fn new(text: &'a [u8], lookup: Lookup<'a>) -> Self {
        let at = if text.starts_with(b"\xEF\xBB\xBF") { 3 } else { 0 }; // a byte order mark
        Reader { s: text, lookup, at, known: BTreeMap::new() }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn read(mut self) -> Vec<Assignment> {
        let mut out = Vec::new();
        while self.at < self.s.len() {
            let begin = self.at;
            self.skip_blanks();
            let Some(c) = self.peek() else { break };
            if is_newline(c) {
                self.end_line();
                continue;
            }
            if c == b'#' {
                self.skip_line();
                continue;
            }
            if let Some(mut assignment) = self.statement() {
                assignment.begin = begin;
                assignment.end = self.at;
                self.known.insert(assignment.name.clone(), assignment.value.clone());
                out.push(assignment);
            }
        }
        out
    }

    /// A statement, from its name to past the end of its last line.
    fn statement(&mut self) -> Option<Assignment> {
        if self.s[self.at..].starts_with(b"export") && self.blank_at(self.at + 6) > 0 {
            self.at += 6;
            self.skip_blanks();
        }
        let mut name = Vec::new();
        while let Some(c) = self.peek() {
            if c == b'=' || c == b'#' || text::space_at(self.s, self.at) > 0 {
                break;
            }
            name.push(c);
            self.at += 1;
        }
        self.skip_blanks();
        if name.is_empty() || self.peek() != Some(b'=') {
            self.skip_line(); // no name, or a name without a value
            return None;
        }
        self.at += 1;
        let value_start = self.at;
        self.skip_blanks();
        let value = match self.peek() {
            Some(b'\'') => self.quoted(b'\'')?,
            Some(b'"') => {
                let value = self.quoted(b'"')?;
                self.expand(&value)
            }
            _ => {
                self.at = value_start; // the blanks before the value count for a comment that begins it
                let value = self.unquoted();
                self.skip_line();
                self.expand(&value)
            }
        };
        Some(Assignment { name, value, begin: 0, end: 0 })
    }

    /// From the opening quote to the closing one, then only blanks and a comment may follow on the line.
    fn quoted(&mut self, quote: u8) -> Option<Vec<u8>> {
        let start = self.at;
        self.at += 1;
        let mut value = Vec::new();
        while let Some(c) = self.peek().filter(|&c| c != quote) {
            self.at += 1;
            if c == b'\\'
                && let Some(next) = self.peek()
            {
                let escaped = match (quote, next) {
                    (b'\'', b'\\' | b'\'') => Some(next),
                    (b'"', b'\\' | b'\'' | b'"') => Some(next),
                    (b'"', b'a') => Some(b'\x07'),
                    (b'"', b'b') => Some(b'\x08'),
                    (b'"', b'f') => Some(b'\x0c'),
                    (b'"', b'n') => Some(b'\n'),
                    (b'"', b'r') => Some(b'\r'),
                    (b'"', b't') => Some(b'\t'),
                    (b'"', b'v') => Some(b'\x0b'),
                    _ => None,
                };
                if let Some(escaped) = escaped {
                    value.push(escaped);
                    self.at += 1;
                    continue;
                }
            }
            value.push(c);
        }
        if self.at >= self.s.len() {
            // never closed: the line is not read
            self.at = start;
            self.skip_line();
            return None;
        }
        self.at += 1;
        self.skip_blanks();
        let clean = self.peek().is_none_or(|c| is_newline(c) || c == b'#');
        self.skip_line();
        if !clean {
            return None; // something after the closing quote
        }
        Some(value)
    }

    /// To the line's end, whitespace around taken off; a `#` after whitespace begins a comment.
    fn unquoted(&mut self) -> Vec<u8> {
        let start = self.at;
        while self.peek().is_some_and(|c| !is_newline(c)) {
            self.at += 1;
        }
        let mut line = &self.s[start..self.at];
        let mut after_space = false;
        let mut i = 0;
        while i < line.len() {
            if line[i] == b'#' && after_space {
                line = &line[..i];
                break;
            }
            let length = text::space_at(line, i);
            after_space = length > 0;
            i += if after_space { length } else { 1 };
        }
        text::strip(line).to_vec()
    }

    /// ${NAME} and ${NAME:-default}, as python-dotenv takes them: a name of anything but `}` and `:`.
    fn expand(&self, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < value.len() {
            let Some((open, close)) = find(value, b"${", i).and_then(|open| Some((open, find(value, b"}", open + 2)?)))
            else {
                out.extend_from_slice(&value[i..]);
                break;
            };
            out.extend_from_slice(&value[i..open]);
            let inner = &value[open + 2..close];
            let colon = inner.iter().position(|&c| c == b':');
            if let Some(colon) = colon
                && !inner[colon..].starts_with(b":-")
            {
                out.extend_from_slice(&value[open..=close]); // not a variable: as it is
                i = close + 1;
                continue;
            }
            let (name, fallback) = match colon {
                Some(colon) => (&inner[..colon], &inner[colon + 2..]),
                None => (inner, &b""[..]),
            };
            if let Some(known) = self.known.get(name) {
                out.extend_from_slice(known);
            } else if let Some(found) = (self.lookup)(name) {
                out.extend_from_slice(&found);
            } else {
                out.extend_from_slice(fallback);
            }
            i = close + 1;
        }
        out
    }

    /// The length of the whitespace character at `at` that does not end a line (python-dotenv's [^\S\r\n]), 0 if
    /// there is none there.
    fn blank_at(&self, at: usize) -> usize {
        if self.s.get(at).is_some_and(|&c| is_newline(c)) { 0 } else { text::space_at(self.s, at) }
    }

    fn skip_blanks(&mut self) {
        loop {
            let length = self.blank_at(self.at);
            if length == 0 {
                break;
            }
            self.at += length;
        }
    }

    fn skip_line(&mut self) {
        while self.peek().is_some_and(|c| !is_newline(c)) {
            self.at += 1;
        }
        self.end_line();
    }

    /// Past one line end: \r\n, \r or \n.
    fn end_line(&mut self) {
        if self.peek() == Some(b'\r') {
            self.at += 1;
        }
        if self.peek() == Some(b'\n') {
            self.at += 1;
        }
    }
}

/// The assignments of a file, in order (a later one of a name wins). What it reads:
///   - a UTF-8 byte order mark at the start; lines ended by \n, \r\n or \r; whitespace as Python's str.isspace()
///     takes it (python-dotenv's \s), U+3000 and the others beyond ASCII too;
///   - blank lines and lines that begin with `#`; an `export ` before a name;
///   - unquoted values, ending at the line's end or at a `#` after whitespace, stripped (`NAME= # note` is empty);
///   - 'single-quoted' values, taken as they are but for \' and \\;
///   - "double-quoted" values with the escapes \\ \' \" \a \b \f \n \r \t \v;
///   - quoted values that go on over several lines;
///   - ${NAME} and ${NAME:-default} in unquoted and double-quoted values: the file's own earlier value of NAME, else
///     `lookup` (the environment), else the default, else nothing. Anything else after a `$` stays as it is.
///
/// A line it cannot read is left out.
pub fn parse(text: &[u8], lookup: Lookup<'_>) -> Vec<Assignment> {
    Reader::new(text, lookup).read()
}

/// `value` written so that `parse` reads it back as it is: bare when it has nothing a reader would take apart, else
/// single-quoted.
pub fn quote(value: &[u8]) -> Vec<u8> {
    let safe = |c: &u8| c.is_ascii_alphanumeric() || b"_.,:/@+=-%~".contains(c);
    if value.iter().all(safe) {
        return value.to_vec();
    }
    let mut out = vec![b'\''];
    for &c in value {
        if c == b'\\' || c == b'\'' {
            out.push(b'\\');
        }
        out.push(c);
    }
    out.push(b'\'');
    out
}

/// The text with NAME's first assignment (all its lines) made the line `NAME=<written>`, its later ones dropped, every
/// other byte kept; a line added at the end when there was none.
pub fn replace(text: &[u8], name: &[u8], written: &[u8]) -> Vec<u8> {
    let line = [name, b"=", written, b"\n"].concat();
    let mut out = Vec::new();
    let mut at = 0;
    let mut done = false;
    for assignment in parse(text, &|_| None) {
        if assignment.name != name {
            continue;
        }
        out.extend_from_slice(&text[at..assignment.begin]);
        if !done {
            out.extend_from_slice(&line); // the first assignment is replaced, a later one dropped
        }
        done = true;
        at = assignment.end;
    }
    out.extend_from_slice(&text[at..]);
    if !done {
        if out.last().is_some_and(|&c| !is_newline(c)) {
            out.push(b'\n');
        }
        out.extend_from_slice(&line);
    }
    out
}
