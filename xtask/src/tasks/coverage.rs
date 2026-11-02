//! Branch coverage of the tests of the core.

use super::{CORE, succeed};
use crate::pins::{LLVM_COV, NIGHTLY};
use crate::record::Run;
use crate::run::{Result, capped, cargo};

pub fn run(cap: u64, run: &Run) -> Result<()> {
    let log = &run.log;
    // --no-report implies --no-clean: the profiles of earlier runs, of code since changed, would be merged in.
    succeed(log.run(cargo(Some(NIGHTLY), &[LLVM_COV.command, "clean", "--workspace"]))?.success(), "cleaning failed")?;
    let test = cargo(Some(NIGHTLY), &[LLVM_COV.command, "--branch", "--no-report", "--locked", "-p", CORE]);
    succeed(log.run(capped(test, cap, false))?.success(), "a test failed")?;
    let report = cargo(Some(NIGHTLY), &[LLVM_COV.command, "report", "--branch", "-p", CORE]);
    succeed(log.run_into(report, &run.dir.join("summary.txt"))?.success(), "cargo llvm-cov report failed")?;
    // Hundreds of files, nearly all changed every time: kept in target/ to look at, not committed.
    let html = cargo(Some(NIGHTLY), &[LLVM_COV.command, "report", "--branch", "-p", CORE, "--html"]);
    succeed(log.run(html)?.success(), "cargo llvm-cov report --html failed")?;
    log.say("coverage: the lines and branches not run are marked in target/llvm-cov/html/index.html");
    Ok(())
}
