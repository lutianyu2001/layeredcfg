//! What each task needs installed, whether it is, and installing it.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::pins::{DENY, FUZZ, GEIGER, KANI, LLVM_COV, NIGHTLY, TOOLS, Tool, VET};
use crate::run::{Result, cargo};

/// The components of the nightly: Miri and the source of the standard library for Miri, the LLVM tools for the
/// coverage.
const COMPONENTS: [&str; 3] = ["miri", "rust-src", "llvm-tools"];

/// The tasks, in the order `all` runs them: the cheap ones first.
pub const TASKS: [&str; 7] = ["deny", "vet", "geiger", "miri", "coverage", "kani", "fuzz"];

type Needs = (Vec<Tool>, Option<Vec<&'static str>>);

/// What a task runs: its tools, and the components of the nightly if it runs the nightly; None for what is no task.
fn uses(task: &str) -> Option<Needs> {
    Some(match task {
        "deny" => (vec![DENY], None),
        "vet" => (vec![VET], None),
        "geiger" => (vec![GEIGER], None),
        "miri" => (vec![], Some(vec!["miri", "rust-src"])),
        "coverage" => (vec![LLVM_COV], Some(vec!["llvm-tools"])),
        "kani" => (vec![KANI], None),
        "fuzz" => (vec![FUZZ], Some(vec![])),
        _ => return None,
    })
}

/// What a task needs installed: what it runs, and for the fuzzing cargo deny too, which checks the dependencies of the
/// fuzz targets before they are built.
fn needs(task: &str) -> Option<Needs> {
    if task == "all" {
        return Some((TOOLS.to_vec(), Some(COMPONENTS.to_vec())));
    }
    let (mut tools, nightly) = uses(task)?;
    if task == "fuzz" {
        tools.push(DENY);
    }
    Some((tools, nightly))
}

/// The pinned versions `task` runs, as run.txt keeps them: "cargo-fuzz 0.13.2, nightly-2026-10-28".
pub fn versions(task: &str) -> String {
    let (tools, nightly) = uses(task).unwrap_or_default();
    let mut versions: Vec<String> = tools.iter().map(|tool| format!("{} {}", tool.krate, tool.version)).collect();
    if nightly.is_some() {
        versions.push(NIGHTLY.to_owned());
    }
    versions.join(", ")
}

/// The version of `tool` cargo install has installed, if any. From cargo's list and not from the tool, which may do
/// more than say its version (`cargo kani --version` sets Kani up first, a large download).
fn installed(tool: Tool) -> Option<String> {
    let output = cargo(None, &["install", "--list"]).output().ok()?;
    // "cargo-deny v0.20.2:" for each crate, then its programs indented.
    String::from_utf8_lossy(&output.stdout).lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (!line.starts_with(char::is_whitespace) && words.next() == Some(tool.krate))
            .then(|| words.next()?.trim_end_matches(':').strip_prefix('v').map(str::to_owned))
            .flatten()
    })
}

/// Whether Kani has set up its own toolchain and CBMC, which `cargo kani setup` does.
fn kani_set_up() -> bool {
    let home = env::var_os("KANI_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".kani")));
    home.is_some_and(|home| home.join(format!("kani-{}", KANI.version)).is_dir())
}

/// Whether the nightly is installed with `components`.
fn nightly_ready(components: &[&str]) -> bool {
    let Ok(output) = Command::new("rustup").args(["component", "list", "--installed", "--toolchain", NIGHTLY]).output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    // "rust-src", "miri-x86_64-unknown-linux-gnu"
    output.status.success()
        && components
            .iter()
            .all(|component| text.lines().any(|line| line == *component || line.starts_with(&format!("{component}-"))))
}

/// What is missing for `task`, as lines to show; none if it can run.
pub fn missing(task: &str) -> Vec<String> {
    let (tools, nightly) = needs(task).unwrap_or_default();
    let mut missing = Vec::new();
    for tool in tools {
        match installed(tool) {
            Some(version) if version == tool.version => {}
            Some(version) => missing.push(format!("{} is {version}, not {}", tool.krate, tool.version)),
            None => missing.push(format!("{} {} is not installed", tool.krate, tool.version)),
        }
        if tool == KANI && !kani_set_up() {
            missing.push(format!("Kani {} is not set up", KANI.version));
        }
    }
    if let Some(components) = nightly
        && !nightly_ready(&components)
    {
        let with = if components.is_empty() { String::new() } else { format!(" with {}", components.join(", ")) };
        missing.push(format!("{NIGHTLY}{with} is not installed"));
    }
    missing
}

/// `cargo xtask setup [task…]`: installs what `tasks` need (every task by default) that is missing or at another
/// version.
pub fn setup(tasks: &[String]) -> Result<()> {
    let tasks = if tasks.is_empty() { vec!["all".to_owned()] } else { tasks.to_vec() };
    let mut tools: Vec<Tool> = Vec::new();
    let mut nightly = false;
    for task in &tasks {
        let (needed, needs_nightly) = needs(task).ok_or_else(|| format!("no task {task}"))?;
        for tool in needed {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
        nightly |= needs_nightly.is_some();
    }
    // Out of the repository, so that no cargo configuration of it applies to the installing; and cargo() leaves out
    // the toolchain rustup set for xtask, which the tools would be built with otherwise.
    let away = env::temp_dir();
    for tool in tools {
        if installed(tool).as_deref() != Some(tool.version) {
            println!("installing {} {}", tool.krate, tool.version);
            let mut install = cargo(None, &["install", "--locked", "--version", tool.version, tool.krate]);
            succeed(install.current_dir(&away))?;
        }
        if tool == KANI && !kani_set_up() {
            succeed(cargo(None, &["kani", "setup"]).current_dir(&away))?;
        }
    }
    if nightly && !nightly_ready(&COMPONENTS) {
        println!("installing {NIGHTLY}");
        if !install_nightly(NIGHTLY) {
            return Err(format!("cannot install {NIGHTLY} with {}", COMPONENTS.join(", ")).into());
        }
    }
    println!("setup: everything {} needs is installed", tasks.join(", "));
    Ok(())
}

/// Installs `toolchain` with the components the nightly needs; false if it cannot (one of them may be missing that
/// day).
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
