//! Kani proofs: for every input up to 6 bytes, UTF-8 is decoded as the standard library does, the readers of
//! whitespace, integers and booleans do not panic or overflow, text is cut only between characters, and lists are cut
//! at their commas; for every ASCII input up to 6 bytes, whitespace is taken off once and for all and integers are
//! read as the standard library reads them; integers are proved at the edges of 64 bits too. `parse_float`, which
//! builds the text it gives str::parse, has no proof. `cargo xtask kani` runs them, one at a time.
//!
//! What is whitespace or a digit beyond ASCII is asked of a function given at run time (for the Python binding, the
//! running Python). Under Kani it answers anything at all, each time it is asked, so that the proofs above that take
//! any bytes hold whatever it says. That a character reads the same each time is no part of it, so what depends on it
//! (stripping twice changes nothing) is proved on ASCII only; the tests and the `numbers` fuzz target check the rest.
//!
//! Kani unrolls every loop of a harness to the one bound its `#[kani::unwind]` gives, so the code under proof is kept
//! without loops where it can be: a table searched in a loop takes the bound of its search into every loop over the
//! input (with a search of 68 runs, harnesses of 6 bytes ran past 30 minutes or past 16G of memory). A harness
//! without a bound unrolls without end. Nor do they push onto a `Vec` in a loop: every push carries the branch that
//! grows it, unrolled at every step.
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

/// Any ASCII bytes, at most N of them.
fn any_ascii<const N: usize>() -> ([u8; N], usize) {
    let (bytes, len) = any_bytes::<N>();
    kani::assume(bytes.iter().all(u8::is_ascii));
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
    let (bytes, len) = any_ascii::<6>();
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

/// Whatever each attempt says (a signal, a buffer too small, another error, no such user, an entry with or without
/// a directory), the retries of the password lookup do exactly what they should: the buffer is never empty nor past
/// the limit, and doubles (up to it) after a buffer too small and only then; a call is made again after a signal or a
/// buffer too small, while the limits allow, and after nothing else; the lookup stops at the first answer it cannot
/// go on from, and gives the directory found, unchanged, or nothing. The limits are small so that every sequence of
/// answers is checked; the real ones (`passwd::LIMITS`) differ only in how many steps there are, which is argued, not
/// proved. The calls into the C library themselves (`attempt`, `read`) are no part of it.
#[kani::proof]
#[kani::unwind(10)]
fn the_password_lookup_retries_end_within_their_limits() {
    use crate::passwd::{Attempt, Limits, retry};
    const MOST_SIZE: usize = 8;
    const MOST_INTERRUPTIONS: u32 = 2;
    let limits = Limits { most_size: MOST_SIZE, most_interruptions: MOST_INTERRUPTIONS };
    let first: usize = kani::any();
    let (mut calls, mut last_size, mut signals) = (0u32, 0usize, 0u32);
    let mut previous: Option<Attempt> = None;
    // Whether retry may go on after `answer`, given the size it was made with and the signals so far, its own too.
    let goes_on = |answer: &Attempt, size: usize, signals: u32| match answer {
        Attempt::Failed(libc::EINTR) => signals <= MOST_INTERRUPTIONS,
        Attempt::Failed(libc::ERANGE) => size < MOST_SIZE,
        _ => false,
    };
    let found = retry(first, &limits, |size| {
        assert!((1..=MOST_SIZE).contains(&size));
        if let Some(answer) = &previous {
            // A call after an answer that allows one, with the size it should have.
            assert!(goes_on(answer, last_size, signals));
            if *answer == Attempt::Failed(libc::ERANGE) {
                assert!(size == (last_size * 2).min(MOST_SIZE));
            } else {
                assert!(size == last_size);
            }
        }
        calls += 1;
        last_size = size;
        let answer = match kani::any::<u8>() % 6 {
            0 => Attempt::Failed(libc::EINTR),
            1 => Attempt::Failed(libc::ERANGE),
            2 => Attempt::Failed(kani::any()),
            3 => Attempt::NoUser,
            4 => Attempt::Found(None),
            _ => Attempt::Found(Some(vec![b'/'])),
        };
        if answer == Attempt::Failed(libc::EINTR) {
            signals += 1;
        }
        previous = Some(answer.clone());
        answer
    });
    // It stopped at an answer it could not go on from, and gives what that answer found.
    let last = previous.expect("retry makes at least one call");
    assert!(!goes_on(&last, last_size, signals));
    let home = match last {
        Attempt::Found(home) => home,
        _ => None,
    };
    assert!(found == home);
    // From 1 byte: three doublings to 8, two calls made again, and the last.
    assert!(calls <= 3 + 2 + 1);
}
