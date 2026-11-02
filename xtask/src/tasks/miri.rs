//! The tests of the core under Miri.

use super::{CORE, succeed};
use crate::pins::NIGHTLY;
use crate::run::{Log, Result, capped, cargo};

/// The tests write files and read the environment.
const MIRI_FLAGS: &str = "-Zmiri-disable-isolation";

pub fn run(cap: u64, log: &Log) -> Result<()> {
    let mut command = cargo(Some(NIGHTLY), &["miri", "test", "--locked", "-p", CORE]);
    command.env("MIRIFLAGS", MIRI_FLAGS);
    succeed(log.run(capped(command, cap, false))?.success(), "a test failed under Miri")
}
