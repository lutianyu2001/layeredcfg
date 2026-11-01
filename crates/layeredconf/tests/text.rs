//! Text to values and back, held to what Python's `int()`, `float()` and `repr()` give.

use layeredconf::text::{format_float, parse_bool, parse_float, parse_int, space_at, split_list, strip, valid_utf8};

fn int(s: &str) -> Option<i64> {
    parse_int(s.as_bytes())
}

fn float(s: &str) -> Option<f64> {
    parse_float(s.as_bytes())
}

#[test]
fn integers_are_read_as_python_int_reads_them() {
    assert_eq!(int("42"), Some(42));
    assert_eq!(int("  -7\n"), Some(-7));
    assert_eq!(int("+0"), Some(0));
    assert_eq!(int("007"), Some(7));
    assert_eq!(int("1_000_000"), Some(1_000_000));
    assert_eq!(int("9223372036854775807"), Some(i64::MAX));
    assert_eq!(int("-9223372036854775808"), Some(i64::MIN));
    for bad in ["", " ", "4x", "84x0", "1.5", "_1", "1_", "1__0", "--1", "0x10", "1 2", "9223372036854775808"] {
        assert_eq!(int(bad), None, "{bad:?}");
    }
}

#[test]
fn floats_are_read_as_python_float_reads_them() {
    assert_eq!(float("0.6"), Some(0.6));
    assert_eq!(float(" 10 "), Some(10.0));
    assert_eq!(float("-.5"), Some(-0.5));
    assert_eq!(float("5."), Some(5.0));
    assert_eq!(float("1e3"), Some(1000.0));
    assert_eq!(float("1_000.5"), Some(1000.5));
    assert_eq!(float("2.5E-1"), Some(0.25));
    assert!(float("Infinity").unwrap().is_infinite());
    assert!(float("-inf").unwrap() < 0.0);
    assert!(float("NaN").unwrap().is_nan());
    // Past the range: infinite or zero, as Python has them.
    assert!(float("1e400").unwrap().is_infinite());
    assert_eq!(float("1e-400"), Some(0.0));
    assert!(float("-1e-400").unwrap().is_sign_negative());
    assert!(float("1e-310").unwrap() > 0.0); // a subnormal number is a number
    // Exponents past any range, without an overflow of their own.
    assert!(float("10e99999999999999999999").unwrap().is_infinite());
    assert!(float("-10e99999999999999999999").unwrap() < 0.0);
    assert!(float("123e9223372036854775807").unwrap().is_infinite());
    assert_eq!(float("0.1e-99999999999999999999"), Some(0.0));
    assert!(float("-nan").unwrap().is_sign_negative());
    for bad in ["", ".", "e5", "1e", "1e+", "soon", "1.5.2", "0x1p3", "1_.5", "1._5", "in f", "--1"] {
        assert_eq!(float(bad), None, "{bad:?}");
    }
}

#[test]
fn digits_and_whitespace_beyond_ascii_are_read_as_python_reads_them() {
    assert_eq!(int("８４２０"), Some(8420)); // full-width digits
    assert_eq!(int("٣٥"), Some(35)); // Arabic-Indic digits
    assert_eq!(int("\u{1D7D3}"), Some(5)); // a mathematical digit, beyond the BMP
    assert_eq!(int("-５"), Some(-5));
    assert_eq!(int("１_０"), Some(10));
    assert_eq!(int("１_0"), Some(10));
    assert_eq!(int("\u{A0}5\u{3000}"), Some(5)); // a no-break space and an ideographic space around
    assert_eq!(int("\u{2028}5\u{85}"), Some(5));
    assert_eq!(float("５.５"), Some(5.5));
    assert_eq!(float("１e５"), Some(100_000.0));
    assert_eq!(float("٣.٥"), Some(3.5));
    assert_eq!(float("\u{3000}-０.５\u{3000}"), Some(-0.5));
    // No other character beyond ASCII (full-width signs, points and letters, a superscript, zero-width and byte order
    // marks), and not \x1c to \x1f, which str.isspace() takes but int() and float() do not.
    let bad: [&[u8]; 12] = [
        b"\x1c5",
        b"5\x1f",
        "＋5".as_bytes(),
        "５．５".as_bytes(),
        "²".as_bytes(),
        "\u{200B}5".as_bytes(),
        "\u{FEFF}5".as_bytes(),
        "\u{180E}5".as_bytes(),
        "1\u{3000}2".as_bytes(),
        "１ｅ５".as_bytes(),
        "ｉｎｆ".as_bytes(),
        b"\xEF\xBC",
    ];
    for bad in bad {
        assert_eq!(parse_int(bad), None, "{bad:?}");
        assert_eq!(parse_float(bad), None, "{bad:?}");
    }
}

#[test]
fn strip_booleans_and_lists_take_off_python_whitespace() {
    assert_eq!(strip("\x1c a \x1f\u{85}\u{3000}".as_bytes()), b"a");
    assert_eq!(strip("\u{200B} a".as_bytes()), "\u{200B} a".as_bytes()); // a zero-width space is no whitespace
    assert!(strip("\u{3000}\u{A0}".as_bytes()).is_empty());
    assert_eq!(strip(b"\xE3\x80 "), b"\xE3\x80"); // not UTF-8: no whitespace
    assert_eq!(space_at("a\u{3000}".as_bytes(), 1), 3);
    assert_eq!(space_at("a\u{3000}".as_bytes(), 0), 0);
    assert_eq!(space_at(b"\xE3\x80", 0), 0);
    assert_eq!(space_at(b"", 0), 0);
    assert_eq!(parse_bool("\u{3000}yes\u{A0}".as_bytes()), Some(true));
    assert_eq!(parse_bool(b"\x1con"), Some(true));
    assert_eq!(parse_bool("\u{3000}".as_bytes()), Some(false));
    assert_eq!(split_list("a.lan\u{3000},\u{A0}b.lan,\u{2028}"), ["a.lan", "b.lan"]);
}

#[test]
fn a_boolean_is_one_of_its_words() {
    for yes in ["1", "true", "TRUE", " yes ", "On"] {
        assert_eq!(parse_bool(yes.as_bytes()), Some(true), "{yes:?}");
    }
    for no in ["", "0", "false", "No", "OFF"] {
        assert_eq!(parse_bool(no.as_bytes()), Some(false), "{no:?}");
    }
    for bad in ["ture", "maybe", "2", "y"] {
        assert_eq!(parse_bool(bad.as_bytes()), None, "{bad:?}");
    }
}

#[test]
fn a_list_is_comma_separated_each_item_stripped_and_the_empty_ones_left_out() {
    assert_eq!(split_list("a.lan, b.lan,"), ["a.lan", "b.lan"]);
    assert!(split_list("").is_empty());
    assert!(split_list(" , ,").is_empty());
}

#[test]
fn floats_are_written_as_python_repr_writes_them() {
    assert_eq!(format_float(45.0), "45.0");
    assert_eq!(format_float(0.0), "0.0");
    assert_eq!(format_float(-0.0), "-0.0");
    assert_eq!(format_float(0.1), "0.1");
    assert_eq!(format_float(0.6), "0.6");
    assert_eq!(format_float(123.456), "123.456");
    assert_eq!(format_float(0.0001), "0.0001");
    assert_eq!(format_float(0.00001), "1e-05");
    assert_eq!(format_float(1e15), "1000000000000000.0");
    assert_eq!(format_float(1e16), "1e+16");
    assert_eq!(format_float(1.5e300), "1.5e+300");
    assert_eq!(format_float(-2.5e-7), "-2.5e-07");
    assert_eq!(format_float(1.0 / 3.0), "0.3333333333333333");
    assert_eq!(format_float(f64::INFINITY), "inf");
    assert_eq!(format_float(f64::NEG_INFINITY), "-inf");
    assert_eq!(format_float(f64::NAN), "nan");
}

#[test]
fn utf8_is_checked() {
    assert!(valid_utf8(b"plain"));
    assert!(valid_utf8(b"\xE4\xB8\xAD\xE6\x96\x87")); // 中文
    assert!(valid_utf8(b"\xF0\x9F\x98\x80")); // an emoji
    assert!(!valid_utf8(b"caf\xE9")); // Latin-1
    assert!(!valid_utf8(b"\xC0\xAF")); // overlong
    assert!(!valid_utf8(b"\xED\xA0\x80")); // a surrogate
    assert!(!valid_utf8(b"\xE4\xB8")); // cut short
    // Bytes that are no character are no whitespace, not even an overlong form of one.
    assert_eq!(strip(b"\xC0\xA0x\xC0\xA0"), b"\xC0\xA0x\xC0\xA0"); // U+0020 in two bytes
    assert_eq!(strip(b"\xE0\x82\xA0x"), b"\xE0\x82\xA0x"); // U+00A0 in three
    assert_eq!(strip(b" \xED\xA0\x80 \xF4\x90\x80\x80 "), b"\xED\xA0\x80 \xF4\x90\x80\x80"); // a surrogate, past U+10FFFF
}
