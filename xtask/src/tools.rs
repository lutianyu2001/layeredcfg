//! The versions of the audit's tools, pinned, and installing them.
//!
//! `cargo xtask outdated` rewrites the constants below when an upgrade is agreed to: keep each on one line.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::run::{Result, cargo};

/// The nightly Rust of cargo fuzz, Miri and the branch coverage. Kani brings a nightly of its own.
pub const NIGHTLY: &str = "nightly-2026-10-28";
/// What the nightly needs: Miri and the source of the standard library for Miri, the LLVM tools for the coverage.
const COMPONENTS: [&str; 3] = ["miri", "rust-src", "llvm-tools"];

#[derive(Clone, Copy)]
pub struct Tool {
    /// The crate that installs it.
    pub krate: &'static str,
    pub version: &'static str,
    /// The cargo subcommand it is run as.
    pub subcommand: &'static str,
}

pub const DENY: Tool = Tool { krate: "cargo-deny", version: "0.20.2", subcommand: "deny" };
pub const VET: Tool = Tool { krate: "cargo-vet", version: "0.10.3", subcommand: "vet" };
pub const GEIGER: Tool = Tool { krate: "cargo-geiger", version: "0.14.0", subcommand: "geiger" };
pub const FUZZ: Tool = Tool { krate: "cargo-fuzz", version: "0.13.2", subcommand: "fuzz" };
pub const LLVM_COV: Tool = Tool { krate: "cargo-llvm-cov", version: "0.9.1", subcommand: "llvm-cov" };
pub const KANI: Tool = Tool { krate: "kani-verifier", version: "0.68.0", subcommand: "kani" };

pub const TOOLS: [Tool; 6] = [DENY, VET, GEIGER, FUZZ, LLVM_COV, KANI];

/// What a task needs: tools, and whether the nightly too.
pub fn needs(task: &str) -> (Vec<Tool>, bool) {
    match task {
        "deny" => (vec![DENY], false),
        "vet" => (vec![VET], false),
        "geiger" => (vec![GEIGER], false),
        "miri" => (vec![], true),
        "coverage" => (vec![LLVM_COV], true),
        "kani" => (vec![KANI], false),
        "fuzz" => (vec![FUZZ], true),
        _ => (TOOLS.to_vec(), true),
    }
}

/// The version of `tool` installed, if any.
fn installed(tool: Tool) -> Option<String> {
    let output = cargo(None, &[tool.subcommand, "--version"]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // "cargo-deny 0.20.2", "Kani Rust Verifier 0.68.0 (cargo plugin)"
    text.split_whitespace().find(|word| word.starts_with(|c: char| c.is_ascii_digit())).map(str::to_owned)
}

/// Whether Kani has set up its own toolchain and CBMC, which `cargo kani setup` does.
fn kani_set_up() -> bool {
    let home = env::var_os("KANI_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".kani")));
    home.is_some_and(|home| home.join(format!("kani-{}", KANI.version)).is_dir())
}

fn nightly_ready() -> bool {
    let Ok(output) = Command::new("rustup").args(["component", "list", "--installed", "--toolchain", NIGHTLY]).output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    // "rust-src", "miri-x86_64-unknown-linux-gnu"
    output.status.success()
        && COMPONENTS
            .iter()
            .all(|component| text.lines().any(|line| line == *component || line.starts_with(&format!("{component}-"))))
}

/// What is missing for `task`, as lines to show; none if it can run.
pub fn missing(task: &str) -> Vec<String> {
    let (tools, nightly) = needs(task);
    let mut missing = Vec::new();
    for tool in tools {
        match installed(tool) {
            Some(version) if version == tool.version => {}
            Some(version) => missing.push(format!("{} is {version}, not {}", tool.krate, tool.version)),
            None => missing.push(format!("{} {} is not installed", tool.krate, tool.version)),
        }
        if tool.krate == KANI.krate && !kani_set_up() {
            missing.push(format!("Kani {} is not set up", KANI.version));
        }
    }
    if nightly && !nightly_ready() {
        missing.push(format!("{NIGHTLY} with {} is not installed", COMPONENTS.join(", ")));
    }
    missing
}

/// Installs what `tasks` need that is missing or at another version.
pub fn setup(tasks: &[String]) -> Result<()> {
    let tasks = if tasks.is_empty() { vec!["all".to_owned()] } else { tasks.to_vec() };
    let mut tools: Vec<Tool> = Vec::new();
    let mut nightly = false;
    for task in &tasks {
        let (needed, needs_nightly) = needs(task);
        for tool in needed {
            if !tools.iter().any(|known| known.krate == tool.krate) {
                tools.push(tool);
            }
        }
        nightly |= needs_nightly;
    }
    // Out of the repository, so that no cargo configuration of it applies to the installing.
    let away = env::temp_dir();
    for tool in tools {
        if installed(tool).as_deref() != Some(tool.version) {
            println!("installing {} {}", tool.krate, tool.version);
            let mut install = cargo(None, &["install", "--locked", "--version", tool.version, tool.krate]);
            succeed(install.current_dir(&away))?;
        }
        if tool.krate == KANI.krate && !kani_set_up() {
            succeed(cargo(None, &["kani", "setup"]).current_dir(&away))?;
        }
    }
    if nightly && !nightly_ready() {
        println!("installing {NIGHTLY}");
        if !install_nightly(NIGHTLY) {
            return Err(format!("cannot install {NIGHTLY} with {}", COMPONENTS.join(", ")).into());
        }
    }
    println!("setup: everything {} needs is installed", tasks.join(", "));
    Ok(())
}

/// Installs `toolchain` with the components the nightly needs; false if it cannot (one of them may be missing that day).
pub fn install_nightly(toolchain: &str) -> bool {
    Command::new("rustup")
        .args(["toolchain", "install", toolchain, "--profile", "minimal", "--component", &COMPONENTS.join(",")])
        .status()
        .is_ok_and(|status| status.success())
}

fn succeed(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if status.success() { Ok(()) } else { Err(format!("{command:?} failed: {status}").into()) }
}
