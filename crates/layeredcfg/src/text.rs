//! Text to values and back, the way Python reads and writes them (so that a binding behaves as Python code did).
//!
//! In ASCII, whitespace is what Python's `str.isspace()` takes (\x1c to \x1f too) and digits are 0 to 9. Beyond ASCII
//! it is what `set_beyond_ascii` was given: the Python binding gives the running Python's `str.isspace()` and
//! `unicodedata.decimal()` (U+3000, U+00A0, ８, ٣), so that text reads as that Python reads it, whatever its Unicode
//! version. Without it, whitespace is Unicode's (`char::is_whitespace`, the same characters as Python's beyond ASCII)
//! and there is no digit beyond ASCII, as `str::parse` has none.

use std::fmt::Write;
use std::sync::OnceLock;

/// What a character beyond ASCII is, in numbers and around text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharKind {
    /// Whitespace: stripped around text, and around a number.
    Space,
    /// A decimal digit, with its value (0 to 9), which a number may be written in.
    Digit(u8),
    /// Neither.
    Other,
}

static BEYOND_ASCII: OnceLock<fn(char) -> CharKind> = OnceLock::new();

/// Says what each character beyond ASCII is, for the whole process and before any text is read: the Python binding
/// gives the running Python's view. Only the first call takes effect; it returns whether this one did.
pub fn set_beyond_ascii(kind: fn(char) -> CharKind) -> bool {
    BEYOND_ASCII.set(kind).is_ok()
}

/// What the character of `code` is; bytes that are no character (`NOT_UTF8`) are none of them.
fn kind(code: u32) -> CharKind {
    if let Ok(byte) = u8::try_from(code)
        && byte.is_ascii()
    {
        return match byte {
            // Python's str.isspace() in ASCII: \t, \n, \x0b, \x0c, \r, \x1c to \x1f and the space.
            b'\t'..=b'\r' | 0x1C..=b' ' => CharKind::Space,
            b'0'..=b'9' => CharKind::Digit(byte - b'0'),
            _ => CharKind::Other,
        };
    }
    let Some(c) = char::from_u32(code) else { return CharKind::Other };
    match beyond_ascii(c) {
        CharKind::Digit(digit) if digit > 9 => CharKind::Other,
        other => other,
    }
}

/// What the function given to `set_beyond_ascii` says of a character beyond ASCII; without one, Unicode's whitespace
/// and no digit.
#[cfg(not(kani))]
fn beyond_ascii(c: char) -> CharKind {
    match BEYOND_ASCII.get() {
        Some(kind) => kind(c),
        None if c.is_whitespace() => CharKind::Space,
        None => CharKind::Other,
    }
}

/// Under Kani, any answer at all, each time it is asked: a proof then holds whatever the function says.
#[cfg(kani)]
fn beyond_ascii(_: char) -> CharKind {
    match kani::any::<u8>() {
        0 => CharKind::Space,
        1 => CharKind::Other,
        _ => CharKind::Digit(kani::any()),
    }
}

fn is_space(code: u32) -> bool {
    kind(code) == CharKind::Space
}

/// The code of bytes that are no UTF-8 character.
pub(crate) const NOT_UTF8: u32 = u32::MAX;

pub(crate) struct Decoded {
    /// `NOT_UTF8` when the bytes are no UTF-8 character.
    pub(crate) code: u32,
    /// In bytes, 1 for bytes that are no character.
    pub(crate) length: usize,
}

/// The UTF-8 character at `at`: no overlong forms, no surrogates, nothing past U+10FFFF.
pub(crate) fn decode(s: &[u8], at: usize) -> Decoded {
    const BAD: Decoded = Decoded { code: NOT_UTF8, length: 1 };
    const LEAST: [u32; 5] = [0, 0, 0x80, 0x800, 0x10000];
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
    if code < LEAST[length] || (0xD800..=0xDFFF).contains(&code) || code > 0x0010_FFFF {
        return BAD;
    }
    Decoded { code, length }
}

/// The text as Python's `int()` and `float()` read it: a whitespace character beyond ASCII made a space and a decimal
/// digit beyond ASCII the ASCII digit of its value (Python's `_PyUnicode_TransformDecimalAndSpaceToASCII`); nothing
/// when there is another character beyond ASCII, which neither reads.
fn ascii_number(s: &[u8]) -> Option<Vec<u8>> {
    // Written into one allocation of the text's length (no character becomes more than one byte) rather than pushed,
    // whose growing Kani would unroll at every step.
    let mut out = vec![0; s.len()];
    let mut length = 0;
    let mut at = 0;
    while at < s.len() {
        let c = decode(s, at);
        out[length] = if c.code < 0x80 {
            s[at]
        } else {
            match kind(c.code) {
                CharKind::Space => b' ',
                CharKind::Digit(digit) => b'0' + digit,
                CharKind::Other => return None,
            }
        };
        length += 1;
        at += c.length;
    }
    out.truncate(length);
    Some(out)
}

/// The whitespace `int()` and `float()` take off that text (Python's `Py_ISSPACE`: not \x1c to \x1f, which
/// `str.isspace()` takes).
fn is_number_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x0b')
}

fn strip_number(s: &[u8]) -> &[u8] {
    let begin = s.iter().position(|&c| !is_number_space(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| !is_number_space(c)).map_or(begin, |last| last + 1);
    &s[begin..end]
}

/// Digits with single underscores between them, from `at`, each given to `digit` (the underscores not). False if
/// there is no digit at `at`.
fn digit_run(s: &[u8], at: &mut usize, mut digit: impl FnMut(u8)) -> bool {
    if !s.get(*at).is_some_and(u8::is_ascii_digit) {
        return false;
    }
    digit(s[*at]);
    *at += 1;
    while let Some(&c) = s.get(*at) {
        if c.is_ascii_digit() {
            digit(c);
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
#[must_use]
pub fn space_at(s: &[u8], at: usize) -> usize {
    if at >= s.len() {
        return 0;
    }
    let c = decode(s, at);
    if is_space(c.code) { c.length } else { 0 }
}

/// Where the text is without whitespace at both ends, as Python's `str.strip()` takes it off.
fn strip_range(s: &[u8]) -> std::ops::Range<usize> {
    // In one pass, each character decoded once: from the first that is not whitespace to past the last.
    let mut kept: Option<std::ops::Range<usize>> = None;
    let mut at = 0;
    while at < s.len() {
        let c = decode(s, at);
        if !is_space(c.code) {
            kept = Some(kept.map_or(at, |kept| kept.start)..at + c.length);
        }
        at += c.length;
    }
    kept.unwrap_or(s.len()..s.len())
}

/// As Python's `str.strip()`: whitespace off both ends.
#[must_use]
pub fn strip(s: &[u8]) -> &[u8] {
    &s[strip_range(s)]
}

/// As `strip`, for text that is UTF-8.
#[must_use]
pub fn strip_str(s: &str) -> &str {
    &s[strip_range(s.as_bytes())]
}

/// As Python's int(s) in base 10: whitespace around (but for \x1c to \x1f, which `int()` does not take), a sign, digits
/// with single underscores between them. Past the range of a 64-bit integer is no value.
#[must_use]
pub fn parse_int(raw: &[u8]) -> Option<i64> {
    let ascii = ascii_number(raw)?;
    let s = strip_number(&ascii);
    let mut at = 0;
    let negative = sign(s, &mut at);
    // Accumulated as an unsigned magnitude, which also holds the most negative number; nothing once past u64.
    let mut magnitude = Some(0u64);
    let add = |digit: u8| magnitude = magnitude.and_then(|m| m.checked_mul(10)?.checked_add(u64::from(digit - b'0')));
    if !digit_run(s, &mut at, add) || at != s.len() {
        return None;
    }
    let magnitude = magnitude?;
    if negative { 0i64.checked_sub_unsigned(magnitude) } else { i64::try_from(magnitude).ok() }
}

/// As Python's float(s): whitespace around (as for `int()`), a sign, digits with an optional point and exponent
/// (underscores between digits), or inf, infinity, nan in any case. A number too large is infinite and one too small
/// zero, as in Python.
#[must_use]
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
        let whole = digit_run(s, &mut at, |c| number.push(c));
        let mut fraction = false;
        if s.get(at) == Some(&b'.') {
            number.push(b'.');
            at += 1;
            fraction = digit_run(s, &mut at, |c| number.push(c));
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
            if !digit_run(s, &mut at, |c| number.push(c)) {
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
#[must_use]
pub fn valid_utf8(s: &[u8]) -> bool {
    std::str::from_utf8(s).is_ok()
}

/// 1, true, yes, on and 0, false, no, off or nothing, in any case; anything else is no boolean.
#[must_use]
pub fn parse_bool(raw: &[u8]) -> Option<bool> {
    match strip(raw).to_ascii_lowercase().as_slice() {
        b"1" | b"true" | b"yes" | b"on" => Some(true),
        b"" | b"0" | b"false" | b"no" | b"off" => Some(false),
        _ => None,
    }
}

/// Comma-separated items, each stripped, empty ones left out.
pub fn split_list(s: &str) -> Vec<String> {
    comma_separated(s).map(strip_str).filter(|item| !item.is_empty()).map(str::to_owned).collect()
}

/// The text between commas, as it is.
pub(crate) fn comma_separated(s: &str) -> impl Iterator<Item = &str> {
    // The commas found byte by byte (a comma is ASCII, so each cut falls between characters) rather than by
    // str::split, whose memchr reads a word at a time and is past what Kani proves in time.
    let mut start = 0;
    (0..=s.len()).filter(|&at| s.as_bytes().get(at).is_none_or(|&c| c == b',')).map(move |end| {
        let part = &s[start..end];
        start = end + 1;
        part
    })
}

/// As Python's repr(float): the shortest digits that read back as the same number, written without an exponent from
/// 1e-4 up to 1e16 (with a ".0" when there is no point), with one (e+16, e-05) otherwise; inf, -inf, nan.
#[must_use]
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
            out.extend(std::iter::repeat_n('0', (point - count).unsigned_abs() as usize));
            out += ".0";
        } else {
            let (whole, fraction) = digits.split_at(point.unsigned_abs() as usize);
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
        // Writing to a String does not fail.
        let _ = write!(out, "e{}{:02}", if exponent < 0 { '-' } else { '+' }, exponent.unsigned_abs());
    }
    out
}
