//! Env-style files: reading them, and replacing one name's line.

use std::collections::BTreeMap;

use layeredconf::envfile::{parse, quote, replace};

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("UTF-8")
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|&(name, value)| (name.to_owned(), value.to_owned())).collect()
}

/// The values of a file, a later assignment of a name winning, `${NAME}` taken from `environment`.
fn read_with(source: impl AsRef<[u8]>, environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    let environment = map(environment);
    let lookup = |name: &[u8]| environment.get(std::str::from_utf8(name).ok()?).map(|value| value.clone().into_bytes());
    parse(source.as_ref(), &lookup).into_iter().map(|a| (text(a.name), text(a.value))).collect()
}

fn read(source: impl AsRef<[u8]>) -> BTreeMap<String, String> {
    read_with(source, &[])
}

fn replaced(source: impl AsRef<[u8]>, name: &str, written: impl AsRef<[u8]>) -> String {
    text(replace(source.as_ref(), name.as_bytes(), written.as_ref()).expect("the line can be written"))
}

#[test]
fn names_values_comments_and_export() {
    let values = read(concat!(
        "# ntfy\n",
        "APP_TOPIC=secret\n",
        "export APP_PORT = 8421\n",
        "\n",
        "   APP_HOSTS=a.lan,b.lan   # the hosts\n",
        "APP_HASH=a#b\n",
        "APP_EMPTY=\n",
        "APP_BARE\n",
        "=nameless\n",
    ));
    assert_eq!(
        values,
        map(&[
            ("APP_TOPIC", "secret"),
            ("APP_PORT", "8421"),
            ("APP_HOSTS", "a.lan,b.lan"),
            ("APP_HASH", "a#b"),
            ("APP_EMPTY", ""),
        ])
    );
}

#[test]
fn a_later_assignment_wins_crlf_line_ends_are_read() {
    assert_eq!(read("A=1\r\nA=2\r\n"), map(&[("A", "2")]));
}

#[test]
fn quoted_values() {
    let values = read(concat!(
        "S='it''s'\n",
        "Q='a \\'b\\' # c' # a comment\n",
        "D=\"tab\\there\\n\\\"q\\\" \\x\"\n",
        "M=\"two\nlines\"\n",
        "BAD='open\n",
        "AFTER=\"x\" y\n",
        "OK=1\n",
    ));
    assert!(!values.contains_key("S")); // something after the closing quote
    assert_eq!(values["Q"], "a 'b' # c");
    assert_eq!(values["D"], "tab\there\n\"q\" \\x");
    assert_eq!(values["M"], "two\nlines");
    assert!(!values.contains_key("BAD"));
    assert!(!values.contains_key("AFTER"));
    assert_eq!(values["OK"], "1");
}

#[test]
fn a_reference_takes_the_file_own_earlier_value_then_the_environment_then_the_default() {
    let values = read_with(
        concat!(
            "HOME_DIR=/home/u\n",
            "DATA=${HOME_DIR}/data\n",
            "FROM_ENV=${SHELL}\n",
            "MISSING=[${NOPE}]\n",
            "DEFAULTED=${NOPE:-x}\n",
            "SINGLE='${HOME_DIR}'\n",
            "DOUBLE=\"${HOME_DIR}!\"\n",
        ),
        &[("SHELL", "/bin/bash"), ("HOME_DIR", "/elsewhere")],
    );
    assert_eq!(values["DATA"], "/home/u/data");
    assert_eq!(values["FROM_ENV"], "/bin/bash");
    assert_eq!(values["MISSING"], "[]");
    assert_eq!(values["DEFAULTED"], "x");
    assert_eq!(values["SINGLE"], "${HOME_DIR}");
    assert_eq!(values["DOUBLE"], "/home/u!");
}

#[test]
fn whitespace_beyond_ascii_is_whitespace_as_python_dotenv_takes_it() {
    let values = read(concat!(
        "A=1\u{3000}# note\n",
        "\u{3000}B=2\n",
        "C\u{3000}=\u{3000} 3\u{A0}\n",
        "export\u{3000}D=4\n",
        "E='x'\u{3000}# note\n",
        "F=1\x1c#note\n",
        "G=\u{3000}#note\n",
        "H=1\u{85}I=2\u{2028}\n",
        "J\u{A0}K=1\n",
    ));
    assert_eq!(
        values,
        map(&[("A", "1"), ("B", "2"), ("C", "3"), ("D", "4"), ("E", "x"), ("F", "1"), ("G", ""), ("H", "1\u{85}I=2"),])
    );
    assert_eq!(replaced("\u{3000}A=1\n", "A", "2"), "A=2\n");
}

#[test]
fn a_value_is_written_so_that_it_reads_back_as_it_is() {
    assert_eq!(quote(b"45.0"), b"45.0");
    assert_eq!(quote(b"a.lan,b.lan"), b"a.lan,b.lan");
    assert_eq!(quote(b""), b"");
    for value in
        ["two words", "a#b", "it's", "back\\slash", "${X}", "two\nlines", "\"q\"", "\u{3000}padded\u{3000}", "a\x1c"]
    {
        let line = [&b"V="[..], &quote(value.as_bytes()), b"\n"].concat();
        assert_eq!(read(line)["V"], value, "{value:?}");
    }
}

#[test]
fn a_name_line_is_replaced_later_ones_dropped_every_other_line_kept() {
    let source = "# ntfy\nAPP_TOPIC=secret\nexport APP_LIMIT=10\nAPP_LIMITS=x\nAPP_LIMIT = 12\n";
    assert_eq!(replaced(source, "APP_LIMIT", "45.0"), "# ntfy\nAPP_TOPIC=secret\nAPP_LIMIT=45.0\nAPP_LIMITS=x\n");
    assert_eq!(replaced("A=1", "B", "2"), "A=1\nB=2\n");
    assert_eq!(replaced("", "B", "2"), "B=2\n");
    assert_eq!(replaced("A=1\r\n", "A", "3"), "A=3\n");
}

#[test]
fn a_comment_after_the_equals_sign_a_byte_order_mark_other_line_ends_form_feeds() {
    assert_eq!(
        read("A=   # fill in later\nB=#not a comment\nC= \t# note\n"),
        map(&[("A", ""), ("B", "#not a comment"), ("C", "")])
    );
    assert_eq!(read(b"\xEF\xBB\xBFA=1\nB=2\n"), map(&[("A", "1"), ("B", "2")]));
    assert_eq!(read("A=1\rB=2\rC=3"), map(&[("A", "1"), ("B", "2"), ("C", "3")]));
    assert_eq!(read("\x0cA=1\n"), map(&[("A", "1")]));
    assert_eq!(read("A=${B:C}\nD=$E ${F:-x}\n"), map(&[("A", "${B:C}"), ("D", "$E x")]));
    // ${A} is the value of the latest A above it.
    assert_eq!(read("A=1\nA=2\nB=${A}\nA=3\n"), map(&[("A", "3"), ("B", "2")]));
}

#[test]
fn replace_takes_a_quoted_value_lines_as_one_and_keeps_every_other_byte() {
    // A line inside another variable's value is that value's, not an assignment.
    let source = "APP_NOTE=\"first\nAPP_LIMIT=5 in the note\n\"\r\nAPP_LIMIT=0.1\r\n# end\r\n";
    assert_eq!(
        replaced(source, "APP_LIMIT", "0.3"),
        "APP_NOTE=\"first\nAPP_LIMIT=5 in the note\n\"\r\nAPP_LIMIT=0.3\n# end\r\n"
    );
    // A value over several lines is replaced whole.
    let mine = "A=1\nAPP_HOST='line one\nAPP_PORT=1'\nB=2";
    let once = replaced(mine, "APP_HOST", "localhost");
    assert_eq!(once, "A=1\nAPP_HOST=localhost\nB=2");
    assert_eq!(replaced(&once, "C", "3"), "A=1\nAPP_HOST=localhost\nB=2\nC=3\n");
    assert_eq!(
        read(replaced("X=1\n", "APP_HOST", quote(b"two\nAPP_PORT=1"))),
        map(&[("X", "1"), ("APP_HOST", "two\nAPP_PORT=1")])
    );
}

#[test]
fn a_quote_never_closed_at_the_end_is_not_given_the_line_added() {
    // The reader leaves out the line of a quote never closed and reads the lines after it; a line added with that quote
    // in it would close the quote and take in every line between.
    let source = "A='\nB=2\n";
    assert_eq!(read(source), map(&[("B", "2")]));
    assert_eq!(replace(source.as_bytes(), b"C", b"'it''s'"), None);
    assert_eq!(replace(source.as_bytes(), b"C", b"plain").map(text).as_deref(), Some("A='\nB=2\nC=plain\n"));
    // Nor part of it: a value over several lines whose last line would be read as an assignment of the name.
    assert_eq!(replace(b"FUZZ='r", b"FUZZ", b"'x\nFUZZ=\\'r'"), None);
    // In place too.
    assert_eq!(replace(b"A=\"\nB=2", b"B", b"'\"'"), None);
    assert_eq!(replace(b"A=\"\nB=2", b"B", b"3").map(text).as_deref(), Some("A=\"\nB=3\n"));
}

#[test]
fn what_names_put_into_values_is_bounded() {
    // Each line doubles A: 2^40 bytes without a bound. The lines past 16 MiB in all are left out, and A keeps the last
    // value it had; so is B, which would put 8 MiB more. C, which takes nothing, is read.
    let source = format!("A=x\n{}B=${{A}}\nC=3\n", "A=${A}${A}\n".repeat(40));
    let values = read(source);
    assert_eq!(values["A"].len(), 1 << 23);
    assert_eq!((values.get("B"), values["C"].as_str()), (None, "3"));
}
