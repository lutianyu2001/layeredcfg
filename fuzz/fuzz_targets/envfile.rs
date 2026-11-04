//! Env-style files: any bytes are read without a panic, the statements found lie in order inside the text, and a
//! name replaced reads back as it was written.
#![no_main]

use layeredcfg::envfile;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let lookup = |name: &[u8]| name.starts_with(b"X").then(|| b"from the environment".to_vec());
    let mut at = 0;
    for assignment in envfile::parse(data, &lookup) {
        assert!(at <= assignment.begin && assignment.begin < assignment.end && assignment.end <= data.len());
        assert!(!assignment.name.is_empty());
        at = assignment.end;
    }
    // Unless a quote never closed would take it in, which replace refuses.
    let written = envfile::quote(data);
    if let Some(replaced) = envfile::replace(data, b"FUZZ", &written) {
        let read = envfile::parse(&replaced, &|_| None);
        let last = read.into_iter().rev().find(|assignment| assignment.name == b"FUZZ");
        assert_eq!(last.map(|assignment| assignment.value).as_deref(), Some(data));
    }
});
