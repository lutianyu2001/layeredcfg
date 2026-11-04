//! Defaults and values: any TOML text is read or refused without a panic, and so is any value for each setting, from
//! every layer. The input is the TOML text, a NUL, then the value.
#![no_main]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use layeredcfg::{Config, Environment, Options, Raw};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (toml, value) = match data.iter().position(|&c| c == 0) {
        Some(nul) => (&data[..nul], &data[nul + 1..]),
        None => (data, &b""[..]),
    };
    let Ok(toml) = std::str::from_utf8(toml) else { return };
    // ~name in a path is looked up in the password database, which may go over the network (LDAP, NIS): not for
    // every run. ~ and ~/ take HOME from the environment, here the value itself.
    let looks_up_a_user = value.first() == Some(&b'~') && value.get(1).is_some_and(|&c| c != b'/');
    let given = value.to_vec();
    let options = Options {
        prefix: "F_".to_owned(),
        environment: Environment::new(move |_| Some(OsString::from_vec(given.clone()))),
        ..Options::default()
    };
    let Ok(mut config) = Config::parse(toml, options, "fuzz") else { return };
    let _ = config.problems();
    for key in config.keys().to_vec() {
        let _ = config.get(&key);
        let _ = config.shown(&key);
        if !looks_up_a_user {
            let _ = config.path(&key);
        }
        let _ = config.cast(&key, Raw::Text(OsString::from_vec(value.to_vec())));
        if config.set(&key, Raw::Text(OsString::from_vec(value.to_vec()))).is_ok() {
            assert!(config.get(&key).is_ok());
        }
    }
    let _ = config.overrides();
});
