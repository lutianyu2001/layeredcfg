//! Characters beyond ASCII as `set_beyond_ascii` says, as the Python binding has the running Python say them. A test
//! program of its own: what it sets holds for the whole process.

use layeredcfg::text::{CharKind, parse_float, parse_int, set_beyond_ascii, split_list, strip};

/// A few runs of Unicode's decimal digits and its whitespace; and U+2460 (①), said to be a digit of 10, which no
/// decimal digit is.
fn kind(c: char) -> CharKind {
    let code = u32::from(c);
    for zero in [0x0660, 0xFF10, 0x1D7CE] {
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

fn int(s: &str) -> Option<i64> {
    parse_int(s.as_bytes())
}

fn float(s: &str) -> Option<f64> {
    parse_float(s.as_bytes())
}

#[test]
fn digits_and_whitespace_beyond_ascii_are_read_as_set() {
    assert!(set_beyond_ascii(kind));
    assert!(!set_beyond_ascii(|_| CharKind::Other)); // the first one holds
    assert_eq!(int("８４２０"), Some(8420)); // full-width digits
    assert_eq!(int("٣٥"), Some(35)); // Arabic-Indic digits
    assert_eq!(int("\u{1D7D3}"), Some(5)); // a mathematical digit, beyond the BMP
    assert_eq!(int("-５"), Some(-5));
    assert_eq!(int("１_０"), Some(10));
    assert_eq!(int("１_0"), Some(10));
    assert_eq!(int("\u{A0}５\u{3000}"), Some(5));
    assert_eq!(float("５.５"), Some(5.5));
    assert_eq!(float("１e５"), Some(100_000.0));
    assert_eq!(float("٣.٥"), Some(3.5));
    assert_eq!(float("\u{3000}-０.５\u{3000}"), Some(-0.5));
    // A digit only where a digit can be, and no value past 9.
    for bad in ["＋5", "５．５", "１ｅ５", "ｉｎｆ", "1\u{3000}2", "\u{2460}", "1\u{2460}"] {
        assert_eq!(int(bad), None, "{bad:?}");
        assert_eq!(float(bad), None, "{bad:?}");
    }
    assert_eq!(strip("\u{3000}５\u{A0}".as_bytes()), "５".as_bytes());
    assert_eq!(split_list("٣,\u{2028}a\u{3000}"), ["٣", "a"]);
}
