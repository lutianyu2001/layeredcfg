use std::path::Path;

use crate::run::{capped, nightly, status};
use crate::tasks::limit;

// Per project: the modules, the features and the skipped tests.
/// The modules whose tests Miri runs: those that start no process, open no socket and
/// wait on no worker thread. The names end in `::`, since test filters match parts of
/// names, and an interface test whose name merely contains one would time out.
///
/// The core crate's tests are all integration tests, whose names have no module, and none
/// of them starts a process or opens a socket: the empty filter runs every one.
const MODULES: &[&str] = &[""];
/// What the tests are built with: the core crate's test programs, a new one too. Not its
/// doctests, nor the binding, a cdylib on Python's C API, which Miri cannot run (the
/// Python tests cover it).
const BUILD: &[&str] = &["-p", "layeredconf", "--tests"];
/// Tests too slow under Miri.
const SKIP: &[&str] = &[];

pub fn run(root: &Path) -> Result<(), String> {
    // Filters that match nothing run nothing, and pass.
    if MODULES.is_empty() {
        return Err("no module to test: list them in xtask/src/tasks/miri.rs".to_owned());
    }
    let mut miri = nightly(root, &["miri", "test", "--locked"]);
    miri.args(BUILD).arg("--").args(MODULES);
    for test in SKIP {
        miri.args(["--skip", test]);
    }
    // Tests read and write temporary files and /dev/urandom.
    miri.env("MIRIFLAGS", "-Zmiri-disable-isolation");
    status(capped(miri, &limit("miri")))
}
