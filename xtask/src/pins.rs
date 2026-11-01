//! The versions the audit's tools are pinned at.
//!
//! `cargo xtask outdated` rewrites the constants below when an upgrade is agreed to: keep each on one line. This file is
//! an input of no task (see `record::affects`): a version changed makes stale only the results of the tasks that use
//! the tool, by the versions their run.txt keeps.

/// The nightly Rust of cargo fuzz, Miri and the branch coverage. Kani brings a nightly of its own.
pub const NIGHTLY: &str = "nightly-2026-10-28";

#[derive(Clone, Copy, PartialEq)]
pub struct Tool {
    /// The crate that installs it.
    pub krate: &'static str,
    pub version: &'static str,
    /// The cargo subcommand it is run by.
    pub command: &'static str,
}

pub const DENY: Tool = Tool { krate: "cargo-deny", version: "0.20.2", command: "deny" };
pub const VET: Tool = Tool { krate: "cargo-vet", version: "0.10.3", command: "vet" };
pub const GEIGER: Tool = Tool { krate: "cargo-geiger", version: "0.14.0", command: "geiger" };
pub const FUZZ: Tool = Tool { krate: "cargo-fuzz", version: "0.13.2", command: "fuzz" };
pub const LLVM_COV: Tool = Tool { krate: "cargo-llvm-cov", version: "0.9.1", command: "llvm-cov" };
pub const KANI: Tool = Tool { krate: "kani-verifier", version: "0.68.0", command: "kani" };

pub const TOOLS: [Tool; 6] = [DENY, VET, GEIGER, FUZZ, LLVM_COV, KANI];
