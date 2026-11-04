use std::path::Path;

use crate::run::{capped, nightly, status};
use crate::tasks::limit;

// Per project: the modules, the features and the skipped tests.
/// The modules whose tests Miri runs: those that start no process, open no socket and
/// wait on no worker thread. The names end in `::`, since test filters match parts of
/// names, and an interface test whose name merely contains one would time out. Where a
/// package's tests are all integration tests, whose names have no module, BUILD picks
/// their programs (`-p <crate> --tests`) and `""` runs every test in them.
const MODULES: &[&str] = &[""];
/// What the tests are built with: `--lib`, or the integration tests (see MODULES).
pub const BUILD: &[&str] = &["-p", "layeredconf", "--tests"];
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
