//! Text to values: any bytes are read without a panic, and a number read is written so that it reads back the same.
//! Characters beyond ASCII are read as the Python binding has them read: through `set_beyond_ascii`, here with a few
//! runs of decimal digits, Unicode's whitespace, and one character said to be a digit of 10, which none is.
#![no_main]

use layeredcfg::text::{self, CharKind};
use libfuzzer_sys::fuzz_target;

fn kind(c: char) -> CharKind {
    let code = u32::from(c);
    // Arabic-Indic, Devanagari, full-width and mathematical bold digits.
    for zero in [0x0660, 0x0966, 0xFF10, 0x1D7CE] {
        if (zero..zero + 10).contains(&code) {
            return CharKind::Digit(u8::try_from(code - zero).unwrap_or(u8::MAX));
        }
    }
    match c {
        '\u{2460}' => CharKind::Digit(10),
        c if c.is_whitespace() => CharKind::Space,
        _ => CharKind::Other,
    }
}

fuzz_target!(|data: &[u8]| {
    text::set_beyond_ascii(kind);
    if let Some(value) = text::parse_int(data) {
        assert_eq!(text::parse_int(value.to_string().as_bytes()), Some(value));
    }
    if let Some(value) = text::parse_float(data) {
        let back = text::parse_float(text::format_float(value).as_bytes()).expect("a float written reads back");
        assert!(back.to_bits() == value.to_bits() || (back.is_nan() && value.is_nan()));
    }
    let _ = text::parse_bool(data);
    let stripped = text::strip(data);
    assert_eq!(text::strip(stripped), stripped);
    if let Ok(s) = std::str::from_utf8(data) {
        assert_eq!(text::strip_str(s).as_bytes(), stripped);
        for item in text::split_list(s) {
            assert!(!item.is_empty() && !item.contains(',') && text::strip_str(&item) == item);
        }
    }
});
