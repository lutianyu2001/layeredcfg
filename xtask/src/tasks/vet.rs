//! cargo vet: every dependency audited or exempted.

use super::succeed;
use crate::pins::VET;
use crate::run::{Log, Result, cargo};

pub fn run(log: &Log) -> Result<()> {
    // --locked: without it cargo vet writes supply-chain/imports.lock anew.
    succeed(log.run(cargo(None, &[VET.command, "--locked"]))?.success(), "cargo vet found crates not vetted")
}
