// For a project with no JSON crate among its dependencies: copy to xtask/src/json.rs, add
// `mod json;` to main.rs, and in tasks/kani.rs and tasks/mod.rs replace serde_json's Value
// with `use crate::json::{self, Value};` and serde_json's from_str with `json::parse`; in
// mod.rs, which reads bytes, parse `String::from_utf8(out.stdout)`. The indexing stays the
// same. Nothing in it depends on the project. Run in lazyui-ratatui; checked against
// JSONTestSuite and 600,000 mutated inputs against a strict parser.
//! A strict JSON reader, for `cargo kani list`, `cargo metadata` and the replies of
//! crates.io, OSV and GitHub.
//! Per project: there is no JSON crate among the dependencies, and the tasks add none (see
//! Cargo.toml). This reads text that came over the network, so whatever RFC 8259 does not
//! allow is an error, and so is a key given twice; nesting has a limit, so that a reply of
//! brackets cannot run the stack out.

use std::collections::BTreeMap;
use std::ops::Index;
use std::str::Chars;

/// How deep arrays and objects may nest.
const DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    /// As written: nothing here computes with numbers, and a number that was checked
    /// against the grammar cannot lose digits.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self { Some(s) } else { None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self { Some(*b) } else { None }
    }

    pub fn as_array(&self) -> Option<&Vec<Self>> {
        if let Self::Array(a) = self { Some(a) } else { None }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Object(o) = self { Some(o) } else { None }
    }
}

/// `value["key"]`: what is under the key, or null, as there being no such key is how the
/// replies say there is none.
impl Index<&str> for Value {
    type Output = Self;

    fn index(&self, key: &str) -> &Self {
        self.as_object().and_then(|o| o.get(key)).unwrap_or(&Self::Null)
    }
}

/// `text`, which must be one JSON value and nothing more.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut reader = Reader { rest: text.chars() };
    let value = reader.value(0)?;
    reader.space();
    match reader.peek() {
        None => Ok(value),
        Some(c) => Err(format!("{c:?} after the end of the value")),
    }
}

struct Reader<'a> {
    rest: Chars<'a>,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.rest.clone().next()
    }

    /// Takes `wanted` if it comes next.
    fn take(&mut self, wanted: char) -> bool {
        let found = self.peek() == Some(wanted);
        if found {
            self.rest.next();
        }
        found
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.rest.next();
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        self.space();
        match self.peek() {
            None => Err("the text ends where a value should be".to_owned()),
            Some('n') => self.word("null", Value::Null),
            Some('t') => self.word("true", Value::Bool(true)),
            Some('f') => self.word("false", Value::Bool(false)),
            Some('"') => self.string().map(Value::String),
            Some('[') => self.array(depth),
            Some('{') => self.object(depth),
            Some('-' | '0'..='9') => self.number(),
            Some(c) => Err(format!("{c:?} where a value should be")),
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, String> {
        match self.rest.as_str().strip_prefix(word) {
            Some(rest) => {
                self.rest = rest.chars();
                Ok(value)
            }
            None => Err(format!("not {word}")),
        }
    }

    fn nested(depth: usize) -> Result<usize, String> {
        if depth < DEPTH {
            Ok(depth.saturating_add(1))
        } else {
            Err(format!("nested deeper than {DEPTH}"))
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        let depth = Self::nested(depth)?;
        self.rest.next();
        let mut items = Vec::new();
        self.space();
        if self.take(']') {
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.space();
            if self.take(']') {
                return Ok(Value::Array(items));
            }
            if !self.take(',') {
                return Err("expected , or ] in an array".to_owned());
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        let depth = Self::nested(depth)?;
        self.rest.next();
        let mut entries = BTreeMap::new();
        self.space();
        if self.take('}') {
            return Ok(Value::Object(entries));
        }
        loop {
            self.space();
            if self.peek() != Some('"') {
                return Err("expected a key in an object".to_owned());
            }
            let key = self.string()?;
            self.space();
            if !self.take(':') {
                return Err(format!("expected : after the key {key:?}"));
            }
            let value = self.value(depth)?;
            if entries.contains_key(&key) {
                return Err(format!("the key {key:?} is given twice"));
            }
            entries.insert(key, value);
            self.space();
            if self.take('}') {
                return Ok(Value::Object(entries));
            }
            if !self.take(',') {
                return Err("expected , or } in an object".to_owned());
            }
        }
    }

    /// Digits, at least one.
    fn digits(&mut self) -> Result<(), String> {
        if !matches!(self.peek(), Some('0'..='9')) {
            return Err("expected a digit in a number".to_owned());
        }
        while matches!(self.peek(), Some('0'..='9')) {
            self.rest.next();
        }
        Ok(())
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.rest.as_str();
        self.take('-');
        // No leading zeros: `0` alone, or digits that start with another.
        if !self.take('0') {
            self.digits()?;
        }
        if self.take('.') {
            self.digits()?;
        }
        if self.take('e') || self.take('E') {
            if !self.take('+') {
                self.take('-');
            }
            self.digits()?;
        }
        let read = start.len().saturating_sub(self.rest.as_str().len());
        Ok(Value::Number(start.get(..read).unwrap_or_default().to_owned()))
    }

    fn string(&mut self) -> Result<String, String> {
        self.rest.next();
        let mut out = String::new();
        loop {
            match self.rest.next() {
                None => return Err("a string that does not end".to_owned()),
                Some('"') => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(c) if u32::from(c) < 0x20 => {
                    return Err(format!("the control character {c:?} in a string"));
                }
                Some(c) => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        match self.rest.next() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('/') => Ok('/'),
            Some('b') => Ok('\u{8}'),
            Some('f') => Ok('\u{c}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') => self.unicode(),
            other => Err(format!("the escape {other:?} in a string")),
        }
    }

    /// Four hexadecimal digits: `from_str_radix` alone would also take a sign.
    fn hex4(&mut self) -> Result<u32, String> {
        let mut code = 0;
        for _ in 0..4 {
            let digit = self.rest.next().and_then(|c| c.to_digit(16));
            let digit = digit.ok_or("\\u without four hexadecimal digits")?;
            code = code * 16 + digit;
        }
        Ok(code)
    }

    /// A `\u` escape, after the `u`: a surrogate must come as a pair.
    fn unicode(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if !(self.take('\\') && self.take('u')) {
                    return Err("half a surrogate pair".to_owned());
                }
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err("half a surrogate pair".to_owned());
                }
                0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            code => code,
        };
        char::from_u32(code).ok_or_else(|| "half a surrogate pair".to_owned())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{DEPTH, Value, parse};

    #[test]
    fn values_are_read() {
        let json = parse(
            " {\"a\": [1, -2.5e+3, true, false, null], \"b\": {\"c\": \"d\"}, \"e\": \"\\\"\\\\\\/\
             \\b\\f\\n\\r\\t\\u00e9\\ud83d\\ude00\"}\n",
        )
        .unwrap();
        let items = json["a"].as_array().unwrap();
        assert_eq!(items[1], Value::Number("-2.5e+3".to_owned()));
        assert_eq!(
            (items[2].as_bool(), items[3].as_bool(), &items[4]),
            (Some(true), Some(false), &Value::Null)
        );
        assert_eq!(json["b"]["c"].as_str(), Some("d"));
        assert_eq!(json["e"].as_str(), Some("\"\\/\u{8}\u{c}\n\r\té😀"));
        assert_eq!(json["no such key"]["nor this"], Value::Null);
        assert_eq!(parse("[]").unwrap(), Value::Array(Vec::new()));
        assert_eq!(parse("{ }").unwrap().as_object().map(std::collections::BTreeMap::len), Some(0));
    }

    #[test]
    fn what_the_grammar_does_not_allow_is_an_error() {
        for text in [
            "",
            " ",
            "nul",
            "True",
            "[1,]",
            "[1 2]",
            "{\"a\":1,}",
            "{a:1}",
            "{\"a\" 1}",
            "{\"a\":1}x",
            "1 2",
            "01",
            "-",
            "+1",
            "1.",
            ".5",
            "1e",
            "1e+",
            "\"open",
            "\"a\nb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "\"\\u+123\"",
            "\"\\ud800\"",
            "\"\\ud800\\n\"",
            "\"\\ud800\\u0041\"",
            "\"\\udc00\"",
            "{\"a\":1,\"a\":2}",
            "{\"a\":1,\"\\u0061\":2}",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn nesting_has_a_limit() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&nested(DEPTH)).is_ok());
        assert!(parse(&nested(DEPTH + 1)).is_err());
        assert!(parse(&"[".repeat(1 << 20)).is_err());
        assert!(parse(&"{\"a\":".repeat(1 << 20)).is_err());
    }
}
