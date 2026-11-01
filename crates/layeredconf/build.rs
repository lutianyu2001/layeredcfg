// Digits and whitespace beyond ASCII are read as Python reads them, from a table written from the unicodedata of a
// Python: the one PYO3_PYTHON names (for a wheel, the Python it is built for, whose Unicode version it then has), else
// the one PYTHON names, else python3.

use std::env;
use std::path::PathBuf;
use std::process::Command;

// Cargo sets both for a build script, which fails by panicking.
#[allow(clippy::expect_used)]
fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let script = manifest.join("../../tools/unicode_table.py");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("unicode_table.rs");
    let python = env::var_os("PYO3_PYTHON").or_else(|| env::var_os("PYTHON")).unwrap_or_else(|| "python3".into());

    println!("cargo::rerun-if-changed={}", script.display());
    println!("cargo::rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo::rerun-if-env-changed=PYTHON");

    let status = Command::new(&python)
        .arg(&script)
        .arg(&out)
        .status()
        .unwrap_or_else(|error| panic!("cannot run {}: {error}", python.to_string_lossy()));
    assert!(status.success(), "{} {} failed: {status}", python.to_string_lossy(), script.display());
}
