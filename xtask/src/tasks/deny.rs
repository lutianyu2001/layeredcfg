//! cargo deny: advisories, licenses, duplicates and sources, of the workspace and of the fuzz targets.

use crate::pins::DENY;
use crate::run::{Log, Result, cargo};

pub fn run(log: &Log) -> Result<()> {
    // --workspace: every member is a root of the graph, xtask too, which no crate depends on. --locked: cargo deny runs
    // cargo metadata, which would rewrite a lock out of date and pass.
    let main = log.run(cargo(None, &[DENY.command, "--locked", "--workspace", "check"]))?.success();
    // The fuzz targets are not built by a user of the library, but are built and run here.
    let fuzz = log
        .run(cargo(
            None,
            &[DENY.command, "--locked", "--manifest-path", "fuzz/Cargo.toml", "check", "advisories", "sources"],
        ))?
        .success();
    match (main, fuzz) {
        (true, true) => Ok(()),
        (false, true) => Err("cargo deny found problems in the workspace".into()),
        (true, false) => Err("cargo deny found problems in the dependencies of the fuzz targets".into()),
        (false, false) => Err("cargo deny found problems in the workspace and the fuzz targets".into()),
    }
}
