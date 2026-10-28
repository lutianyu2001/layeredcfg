//! Text to values and back, the way Python reads and writes them (so that a binding behaves as Python code did).
//!
//! Whitespace is what Python's str.isspace() takes, beyond ASCII too (U+3000, U+00A0 and the others), and digits are
//! Unicode's decimal digits (８, ٣), both as the unicodedata of the Python the library was built with has them.

include!(concat!(env!("OUT_DIR"), "/unicode_table.rs"));

/// The code of bytes that are no UTF-8 character.
pub(crate) const NOT_UTF8: u32 = u32::MAX;

pub(crate) struct Decoded {
    /// NOT_UTF8 when the bytes are no UTF-8 character.
    pub(crate) code: u32,
    /// In bytes, 1 for bytes that are no character.
    pub(crate) length: usize,
}

/// The UTF-8 character at `at`: no overlong forms, no surrogates, nothing past U+10FFFF.
pub(crate) fn decode(s: &[u8], at: usize) -> Decoded {
    const BAD: Decoded = Decoded { code: NOT_UTF8, length: 1 };
    let Some(&c) = s.get(at) else { return BAD };
    let length = if c < 0x80 {
        1
    } else if c >> 5 == 0x6 {
        2
    } else if c >> 4 == 0xE {
        3
    } else if c >> 3 == 0x1E {
        4
    } else {
        return BAD;
    };
    let Some(bytes) = s.get(at..at + length) else {
        return BAD;
    };
    let mut code = u32::from(match length {
        1 => c,
        2 => c & 0x1F,
        3 => c & 0x0F,
        _ => c & 0x07,
    });
    for &next in &bytes[1..] {
        if next >> 6 != 0x2 {
            return BAD;
        }
        code = (code << 6) | u32::from(next & 0x3F);
    }
    const LEAST: [u32; 5] = [0, 0, 0x80, 0x800, 0x10000];
    if code < LEAST[length] || (0xD800..=0xDFFF).contains(&code) || code > 0x10FFFF {
        return BAD;
    }
    Decoded { code, length }
}

fn is_space(code: u32) -> bool {
    SPACES.binary_search(&code).is_ok()
}

/// The value of a decimal digit, nothing for a character that is none.
fn decimal(code: u32) -> Option<u8> {
    let after = DIGIT_ZEROS.partition_point(|&zero| zero <= code);
    let zero = DIGIT_ZEROS[after.checked_sub(1)?];
    u8::try_from(code - zero).ok().filter(|&digit| digit <= 9)
}

/// The text as Python's int() and float() read it: a whitespace character beyond ASCII made a space and a decimal
/// digit beyond ASCII the ASCII digit of its value (Python's _PyUnicode_TransformDecimalAndSpaceToASCII); nothing
/// when there is another character beyond ASCII, which neither reads.
fn ascii_number(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut at = 0;
    while at < s.len() {
        if s[at] < 0x80 {
            out.push(s[at]);
            at += 1;
            continue;
        }
        let c = decode(s, at);
        if is_space(c.code) {
            out.push(b' ');
        } else {
            out.push(b'0' + decimal(c.code)?);
        }
        at += c.length;
    }
    Some(out)
}

/// The whitespace int() and float() take off that text (Python's Py_ISSPACE: not \x1c to \x1f, which str.isspace()
/// takes).
fn is_number_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b')
}

fn strip_number(s: &[u8]) -> &[u8] {
    let begin = s.iter().position(|&c| !is_number_space(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| !is_number_space(c)).map_or(begin, |last| last + 1);
    &s[begin..end]
}

/// Digits with single underscores between them, from `at`, appended to `out` without the underscores. False if
/// there is no digit at `at`.
fn digit_run(s: &[u8], at: &mut usize, out: &mut Vec<u8>) -> bool {
    if !s.get(*at).is_some_and(u8::is_ascii_digit) {
        return false;
    }
    out.push(s[*at]);
    *at += 1;
    while let Some(&c) = s.get(*at) {
        if c.is_ascii_digit() {
            out.push(c);
            *at += 1;
        } else if c == b'_' && s.get(*at + 1).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        } else {
            break;
        }
    }
    true
}

/// A sign at `at`, if there is one: whether it is a minus.
fn sign(s: &[u8], at: &mut usize) -> bool {
    match s.get(*at) {
        Some(b'+') => {
            *at += 1;
            false
        }
        Some(b'-') => {
            *at += 1;
            true
        }
        _ => false,
    }
}

/// The length in bytes of the whitespace character at `at`, 0 if there is none there.
pub fn space_at(s: &[u8], at: usize) -> usize {
    if at >= s.len() {
        return 0;
    }
    let c = decode(s, at);
    if is_space(c.code) { c.length } else { 0 }
}

/// Where the text is without whitespace at both ends, as Python's str.strip() takes it off.
fn strip_range(s: &[u8]) -> std::ops::Range<usize> {
    let mut begin = 0;
    loop {
        let length = space_at(s, begin);
        if length == 0 {
            break;
        }
        begin += length;
    }
    let mut end = begin; // past the last character that is not whitespace
    let mut at = begin;
    while at < s.len() {
        let length = space_at(s, at);
        at += if length > 0 { length } else { decode(s, at).length };
        if length == 0 {
            end = at;
        }
    }
    begin..end
}

/// As Python's str.strip(): whitespace off both ends.
pub fn strip(s: &[u8]) -> &[u8] {
    &s[strip_range(s)]
}

/// As `strip`, for text that is UTF-8.
pub fn strip_str(s: &str) -> &str {
    &s[strip_range(s.as_bytes())]
}

/// As Python's int(s) in base 10: whitespace around (but for \x1c to \x1f, which int() does not take), a sign, digits
/// with single underscores between them. Past the range of a 64-bit integer is no value.
pub fn parse_int(raw: &[u8]) -> Option<i64> {
    let ascii = ascii_number(raw)?;
    let s = strip_number(&ascii);
    let mut at = 0;
    let negative = sign(s, &mut at);
    let mut digits = Vec::new();
    if !digit_run(s, &mut at, &mut digits) || at != s.len() {
        return None;
    }
    // Accumulated as an unsigned magnitude, which also holds the most negative number.
    let mut magnitude: u64 = 0;
    for digit in digits {
        magnitude = magnitude.checked_mul(10)?.checked_add(u64::from(digit - b'0'))?;
    }
    if negative { 0i64.checked_sub_unsigned(magnitude) } else { i64::try_from(magnitude).ok() }
}

/// As Python's float(s): whitespace around (as for int()), a sign, digits with an optional point and exponent
/// (underscores between digits), or inf, infinity, nan in any case. A number too large is infinite and one too small
/// zero, as in Python.
pub fn parse_float(raw: &[u8]) -> Option<f64> {
    let ascii = ascii_number(raw)?;
    let s = strip_number(&ascii);
    let mut at = 0;
    let negative = sign(s, &mut at);
    let word = s[at..].to_ascii_lowercase();
    let value = if word == b"inf" || word == b"infinity" {
        f64::INFINITY
    } else if word == b"nan" {
        f64::NAN
    } else {
        let mut number = Vec::new();
        let whole = digit_run(s, &mut at, &mut number);
        let mut fraction = false;
        if s.get(at) == Some(&b'.') {
            number.push(b'.');
            at += 1;
            fraction = digit_run(s, &mut at, &mut number);
        }
        if !whole && !fraction {
            return None;
        }
        if matches!(s.get(at), Some(b'e' | b'E')) {
            number.push(b'e');
            at += 1;
            if let Some(&c @ (b'+' | b'-')) = s.get(at) {
                number.push(c);
                at += 1;
            }
            if !digit_run(s, &mut at, &mut number) {
                return None;
            }
        }
        if at != s.len() {
            return None;
        }
        // Rust reads a number past the range of a double as infinite or zero, as Python does.
        std::str::from_utf8(&number).ok()?.parse::<f64>().ok()?
    };
    Some(if negative { -value } else { value })
}

/// Whether the bytes are UTF-8 (the text of every layer must be).
pub fn valid_utf8(s: &[u8]) -> bool {
    std::str::from_utf8(s).is_ok()
}

/// 1, true, yes, on and 0, false, no, off or nothing, in any case; anything else is no boolean.
pub fn parse_bool(raw: &[u8]) -> Option<bool> {
    match strip(raw).to_ascii_lowercase().as_slice() {
        b"1" | b"true" | b"yes" | b"on" => Some(true),
        b"" | b"0" | b"false" | b"no" | b"off" => Some(false),
        _ => None,
    }
}

/// Comma-separated items, each stripped, empty ones left out.
pub fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(strip_str).filter(|item| !item.is_empty()).map(str::to_owned).collect()
}

/// As Python's repr(float): the shortest digits that read back as the same number, written without an exponent from
/// 1e-4 up to 1e16 (with a ".0" when there is no point), with one (e+16, e-05) otherwise; inf, -inf, nan.
pub fn format_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf" } else { "inf" }.to_owned();
    }
    // Rust writes the shortest digits that read back as the same number: "4.5e1", "1e16", "1e-5".
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let point = exponent + 1; // where the point goes, counted in digits from the first
    let mut out = String::from(if value.is_sign_negative() { "-" } else { "" });
    if point > -4 && point <= 16 {
        let count = i32::try_from(digits.len()).unwrap_or(i32::MAX);
        if point <= 0 {
            out += "0.";
            out.extend(std::iter::repeat_n('0', point.unsigned_abs() as usize));
            out += &digits;
        } else if point >= count {
            out += &digits;
            out.extend(std::iter::repeat_n('0', (point - count) as usize));
            out += ".0";
        } else {
            let (whole, fraction) = digits.split_at(point as usize);
            out += whole;
            out += ".";
            out += fraction;
        }
    } else {
        let (first, rest) = digits.split_at(1);
        out += first;
        if !rest.is_empty() {
            out += ".";
            out += rest;
        }
        out += &format!("e{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.unsigned_abs());
    }
    out
}
