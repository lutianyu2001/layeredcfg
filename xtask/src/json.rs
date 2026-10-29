//! Just enough JSON to read the answers of crates.io and OSV.

#[derive(Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Kept as written: nothing here computes with a number.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }

    pub fn is_true(&self) -> bool {
        *self == Value::Bool(true)
    }
}

/// The value `text` holds, or None if it is not JSON.
pub fn parse(text: &str) -> Option<Value> {
    let mut parser = Parser { s: text.as_bytes(), at: 0, depth: 0 };
    let value = parser.value()?;
    parser.space();
    (parser.at == parser.s.len()).then_some(value)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    /// Arrays and objects open, at most DEEPEST.
    depth: usize,
}

const DEEPEST: usize = 64;

impl Parser<'_> {
    fn space(&mut self) {
        while self.s.get(self.at).is_some_and(|b| b" \t\r\n".contains(b)) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        let found = self.s.get(self.at) == Some(&byte);
        self.at += usize::from(found);
        found
    }

    fn word(&mut self, word: &str, value: Value) -> Option<Value> {
        self.s[self.at..].starts_with(word.as_bytes()).then(|| {
            self.at += word.len();
            value
        })
    }

    fn value(&mut self) -> Option<Value> {
        self.space();
        let first = *self.s.get(self.at)?;
        if first == b'[' || first == b'{' {
            self.depth += 1;
            if self.depth > DEEPEST {
                return None;
            }
        }
        let value = self.inner(first);
        if first == b'[' || first == b'{' {
            self.depth -= 1;
        }
        value
    }

    fn inner(&mut self, first: u8) -> Option<Value> {
        match first {
            b'n' => self.word("null", Value::Null),
            b't' => self.word("true", Value::Bool(true)),
            b'f' => self.word("false", Value::Bool(false)),
            b'"' => self.string().map(Value::String),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if self.eat(b']') {
                            break;
                        }
                        self.eat(b',').then_some(())?;
                    }
                }
                Some(Value::Array(items))
            }
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let name = self.string()?;
                        self.eat(b':').then_some(())?;
                        members.push((name, self.value()?));
                        if self.eat(b'}') {
                            break;
                        }
                        self.eat(b',').then_some(())?;
                    }
                }
                Some(Value::Object(members))
            }
            b'-' | b'0'..=b'9' => {
                // -?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?
                let begin = self.at;
                self.at += usize::from(first == b'-');
                if self.s.get(self.at) == Some(&b'0') {
                    self.at += 1;
                } else {
                    self.digits()?;
                }
                if self.s.get(self.at) == Some(&b'.') {
                    self.at += 1;
                    self.digits()?;
                }
                if self.s.get(self.at).is_some_and(|b| b"eE".contains(b)) {
                    self.at += 1;
                    self.at += usize::from(self.s.get(self.at).is_some_and(|b| b"+-".contains(b)));
                    self.digits()?;
                }
                Some(Value::Number(String::from_utf8_lossy(&self.s[begin..self.at]).into_owned()))
            }
            _ => None,
        }
    }

    /// One digit or more.
    fn digits(&mut self) -> Option<()> {
        let begin = self.at;
        while self.s.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        (self.at > begin).then_some(())
    }

    fn string(&mut self) -> Option<String> {
        (self.s.get(self.at) == Some(&b'"')).then_some(())?;
        self.at += 1;
        let mut bytes = Vec::new();
        loop {
            let byte = *self.s.get(self.at)?;
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(bytes).ok(),
                ..0x20 => return None, // a control character, which must be escaped
                b'\\' => {
                    let escaped = *self.s.get(self.at)?;
                    self.at += 1;
                    let plain = match escaped {
                        b'"' | b'\\' | b'/' => escaped,
                        b'b' => 0x08,
                        b'f' => 0x0C,
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'u' => {
                            let mut units = vec![self.hex4()?];
                            // A surrogate pair is two escapes.
                            if (0xD800..0xDC00).contains(&units[0]) && self.s[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                units.push(self.hex4()?);
                            }
                            for c in char::decode_utf16(units) {
                                bytes.extend_from_slice(c.ok()?.encode_utf8(&mut [0; 4]).as_bytes());
                            }
                            continue;
                        }
                        _ => return None,
                    };
                    bytes.push(plain);
                }
                _ => bytes.push(byte),
            }
        }
    }

    fn hex4(&mut self) -> Option<u16> {
        let digits = self.s.get(self.at..self.at + 4)?;
        self.at += 4;
        // from_str_radix would take a sign
        digits
            .iter()
            .all(u8::is_ascii_hexdigit)
            .then(|| u16::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_crates_io_answers() {
        let text = r#"{"versions": [{"num": "0.13.2", "created_at": "2026-06-09T18:37:27Z", "yanked": false}],
                       "meta": {"total": 1, "next_page": null}}"#;
        let value = parse(text).unwrap();
        let version = &value.get("versions").unwrap().as_array()[0];
        assert_eq!(version.get("num").and_then(Value::as_str), Some("0.13.2"));
        assert!(!version.get("yanked").unwrap().is_true());
        assert_eq!(value.get("meta").and_then(|meta| meta.get("next_page")), Some(&Value::Null));
        assert_eq!(value.get("meta").and_then(|meta| meta.get("total")), Some(&Value::Number("1".into())));
    }

    #[test]
    fn reads_escapes() {
        assert_eq!(parse(r#""a\"\\\/\né😀""#), Some(Value::String("a\"\\/\né😀".into())));
        assert_eq!(parse("{}"), Some(Value::Object(Vec::new())));
        assert_eq!(parse(" [ ] "), Some(Value::Array(Vec::new())));
        assert_eq!(
            parse("[-0.5e+3, 0, 12]"),
            Some(Value::Array(["-0.5e+3", "0", "12"].map(|n| Value::Number(n.into())).into()))
        );
        assert!(parse(&("[".repeat(64) + &"]".repeat(64))).is_some());
    }

    #[test]
    fn refuses_what_is_not_json() {
        let deep = "[".repeat(65) + &"]".repeat(65);
        for text in [
            "",
            "{",
            r#"{"a" 1}"#,
            "[1,]",
            r#""\x""#,
            r#""\ud83d""#,
            "nul",
            "[1] 2",
            "-",
            "1.2.3",
            "1e",
            "01",
            r#""\u+041""#,
            "\"\n\"",
            &deep,
        ] {
            assert_eq!(parse(text), None, "{text}");
        }
    }
}
