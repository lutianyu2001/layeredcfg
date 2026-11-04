//! Text to values: any bytes are read without a panic, and a number read is written so that it reads back the same.
#![no_main]

use layeredcfg::text;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
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
