//! cargo geiger: the unsafe code of the dependencies, counted.

use std::fs;

use super::succeed;
use crate::pins::GEIGER;
use crate::record::Run;
use crate::run::{Result, capped, cargo, root};

pub fn run(cap: u64, run: &Run) -> Result<()> {
    // cargo geiger runs offline, on the sources of the dependencies, which a library does not vendor.
    succeed(run.log.run(cargo(None, &["fetch", "--locked"]))?.success(), "cargo fetch failed")?;
    // From the binding, whose dependencies are all those of the library and more.
    let mut command = cargo(None, &[GEIGER.command, "--locked", "--offline"]);
    command.current_dir(root().join("crates/layeredconf-python"));
    let table = run.dir.join("geiger.txt");
    run.log.run_into(capped(command, cap, false), &table)?;
    // cargo geiger 0.14 ends in failure for files it did not scan (the README the crates include): it ran if it wrote
    // its table. A count to read, not a check.
    succeed(fs::read_to_string(&table)?.contains("Metric output format"), "cargo geiger wrote no table")
}
