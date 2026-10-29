//! Kani proofs: for every input up to 6 bytes, the readers of whitespace, integers, booleans and lists do not panic,
//! do not overflow, and say what the standard library says where it says the same thing; integers are proved at the
//! edges of 64 bits too. `parse_float`, which builds the text it gives str::parse, has no proof. `cargo kani -p
//! layeredconf` runs them.
//!
//! Kani unrolls every loop of a harness to the one bound its `#[kani::unwind]` gives, so the code under proof is kept
//! without loops where it can be. The Unicode lookups are matches of ranges (`tools/unicode_table.py`) and not tables
//! searched: a search of the 68 runs of digits needs a bound of 8, which every loop over the input then takes too,
//! each step reading the table at an unknown place; with them, harnesses of 6 bytes ran past 30 minutes or past 16G of
//! memory, and they take seconds without. A harness without a bound unrolls without end. Nor do they push onto a
//! `Vec` in a loop: every push carries the branch that grows it, unrolled at every step.
//!
//! The reader of env-style files has none: it is a loop over statements with some nine loops inside, each unrolled
//! whole at every step of the outer one, and at 4 bytes it was past 16G of memory. Its tests and the `envfile` fuzz
//! target check it instead.

use crate::text::{self, NOT_UTF8, decode};

/// Any bytes, at most N of them.
fn any_bytes<const N: usize>() -> ([u8; N], usize) {
    let bytes: [u8; N] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= N);
    (bytes, len)
}

#[kani::proof]
#[kani::unwind(7)]
fn decode_reads_utf8_as_the_standard_library_does() {
    let (bytes, len) = any_bytes::<6>();
    let s = &bytes[..len];
    let at: usize = kani::any();
    kani::assume(at < len);
    let decoded = decode(s, at);
    let rest = &s[at..];
    let valid = match std::str::from_utf8(rest) {
        Ok(valid) => valid,
        Err(error) => std::str::from_utf8(&rest[..error.valid_up_to()]).unwrap(),
    };
    match valid.chars().next() {
        Some(c) => assert!(decoded.code == u32::from(c) && decoded.length == c.len_utf8()),
        None => assert!(decoded.code == NOT_UTF8 && decoded.length == 1),
    }
}

#[kani::proof]
#[kani::unwind(7)]
fn decode_past_the_end_is_no_character() {
    let (bytes, len) = any_bytes::<6>();
    let at: usize = kani::any();
    kani::assume(at >= len);
    let decoded = decode(&bytes[..len], at);
    assert!(decoded.code == NOT_UTF8 && decoded.length == 1);
}

#[kani::proof]
#[kani::unwind(7)]
fn space_at_stays_in_the_text() {
    let (bytes, len) = any_bytes::<6>();
    let at: usize = kani::any();
    let length = text::space_at(&bytes[..len], at);
    assert!(length == 0 || at + length <= len);
}

#[kani::proof]
#[kani::unwind(7)]
fn strip_takes_whitespace_off_once_and_for_all() {
    let (bytes, len) = any_bytes::<6>();
    let s = &bytes[..len];
    let stripped = text::strip(s);
    assert!(stripped.len() <= s.len());
    assert!(text::strip(stripped) == stripped);
}

#[kani::proof]
#[kani::unwind(7)]
fn strip_str_cuts_at_the_boundaries_of_characters() {
    let (bytes, len) = any_bytes::<6>();
    if let Ok(s) = std::str::from_utf8(&bytes[..len]) {
        let _ = text::strip_str(s); // slicing a str elsewhere would panic
    }
}

#[kani::proof]
#[kani::unwind(7)]
fn parse_int_does_not_panic_or_overflow() {
    let (bytes, len) = any_bytes::<6>();
    let _ = text::parse_int(&bytes[..len]);
}

#[kani::proof]
#[kani::unwind(7)]
fn parse_int_reads_plain_numbers_as_the_standard_library_does() {
    let (bytes, len) = any_bytes::<6>();
    let s = &bytes[..len];
    // Without whitespace, underscores or digits beyond ASCII, int() and str::parse take the same numbers.
    kani::assume(s.iter().all(|&c| c.is_ascii_digit() || c == b'+' || c == b'-'));
    assert!(text::parse_int(s) == std::str::from_utf8(s).unwrap().parse::<i64>().ok());
}

/// `start`, then any 2 digits: parse_int says what str::parse says. Every byte but those 2 is fixed, so that where
/// the reading is in the text stays known (with the 19 before free too, every loop unrolls for each place, past what
/// the solver does in memory).
fn parse_int_is_exact_after(start: &[u8; 19]) {
    let mut bytes = [0u8; 21];
    bytes[..19].copy_from_slice(start);
    for byte in &mut bytes[19..] {
        *byte = kani::any();
        kani::assume(byte.is_ascii_digit());
    }
    assert!(text::parse_int(&bytes) == std::str::from_utf8(&bytes).unwrap().parse::<i64>().ok());
}

// Where i64 ends (…5807, …5808), and where the magnitude no longer fits u64 (…1615, …1616, and past it in the
// multiplication): through every overflow.

#[kani::proof]
#[kani::unwind(22)]
fn parse_int_is_exact_at_the_largest_i64() {
    parse_int_is_exact_after(b"+092233720368547758");
}

#[kani::proof]
#[kani::unwind(22)]
fn parse_int_is_exact_at_the_smallest_i64() {
    parse_int_is_exact_after(b"-092233720368547758");
}

#[kani::proof]
#[kani::unwind(22)]
fn parse_int_is_exact_past_u64() {
    parse_int_is_exact_after(b"+184467440737095516");
}

#[kani::proof]
#[kani::unwind(22)]
fn parse_int_is_exact_past_u64_negative() {
    parse_int_is_exact_after(b"-184467440737095516");
}

#[kani::proof]
#[kani::unwind(8)]
fn split_list_cuts_at_the_commas_and_nowhere_else() {
    // split_list strips what is between the commas (strip_str, whose proofs are above) and leaves out what is empty;
    // the cutting is what is left to prove. Stripping each part here too would multiply the loops past what the
    // solver does in an hour.
    let (bytes, len) = any_bytes::<6>();
    let Ok(s) = std::str::from_utf8(&bytes[..len]) else {
        return;
    };
    // Each part begins where the one before ended and a comma after it, has no comma, and is followed by a comma or
    // the end; together they are the whole text.
    let (mut at, mut parts) = (0, 0);
    for part in text::comma_separated(s) {
        assert!(at <= s.len() && std::ptr::eq(part.as_ptr(), s.as_bytes()[at..].as_ptr()));
        assert!(part.bytes().all(|c| c != b','));
        assert!(s.as_bytes().get(at + part.len()).is_none_or(|&c| c == b','));
        at += part.len() + 1;
        parts += 1;
    }
    assert!(parts >= 1 && at == s.len() + 1);
}

#[kani::proof]
#[kani::unwind(7)]
fn parse_bool_does_not_panic() {
    let (bytes, len) = any_bytes::<6>();
    let _ = text::parse_bool(&bytes[..len]);
}
