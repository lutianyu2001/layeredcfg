//! The Kani proofs, one at a time, each in a scope of its own where its peak of memory is measured.

use std::fmt::Write as _;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use super::{Kind, Plan, succeed};
use crate::json::{self, Value};
use crate::pins::KANI;
use crate::record::Run;
use crate::run::{Log, Result, capped, cargo, cgroup, memory, no_scope, root, size, this};

const TIMEOUT: &str = "1800s";
/// The cap of a proof without `RUST_XTASK_MEMORY`: the larger of 4G and 1.5 times the largest peak of memory.peak of a
/// proof (scope and all), measured with `RUST_XTASK_MEMORY=16G cargo xtask kani --no-record`; to be measured again
/// when the proofs change. The largest was 10.8 GiB (`parse_int_is_exact_past_u64`, 2026-11-01), 16.2 GiB times 1.5.
const MEMORY: u64 = 17 << 30;

pub fn plan(args: &[String], recorded: bool) -> Result<std::result::Result<Plan, String>> {
    if !args.is_empty() && recorded {
        return Ok(Err("some proofs only may be run with --no-record: they are not the results of all".into()));
    }
    Ok(Ok(Plan { task: "kani", cap: memory(MEMORY)?, kind: Kind::Kani { only: args.to_vec() } }))
}

pub fn run(cap: u64, only: &[String], run: &Run) -> Result<()> {
    let log = &run.log;
    // Lists the proofs of every member, from the root (the manifest is virtual); it takes no cargo arguments.
    let list = cargo(None, &[KANI.command, "list", "--format", "json"]);
    succeed(log.run(capped(list, cap, false))?.success(), "cargo kani list failed")?;
    let listed = run.dir.join("kani-list.json");
    fs::rename(root().join("kani-list.json"), &listed)?;
    let text = fs::read_to_string(&listed)?;
    let value = json::parse(&text).ok_or("kani-list.json is not JSON")?;
    let mut harnesses = Vec::new();
    for kind in ["standard-harnesses", "contract-harnesses"] {
        let Some(Value::Object(files)) = value.get(kind) else {
            return Err(format!("kani-list.json has no {kind}").into());
        };
        for (file, names) in files {
            let krate = package(Path::new(file))?;
            for name in names.as_array().iter().filter_map(Value::as_str) {
                harnesses.push((krate.clone(), name.to_owned()));
            }
        }
    }
    if harnesses.is_empty() {
        return Err("cargo kani list found no harness".into());
    }
    if let Some(unknown) = only.iter().find(|name| !harnesses.iter().any(|(_, harness)| harness == *name)) {
        return Err(format!("no harness {unknown}").into());
    }
    harnesses.retain(|(_, harness)| only.is_empty() || only.contains(harness));
    let mut table = String::new();
    let mut failed = Vec::new();
    for (krate, harness) in &harnesses {
        let mut args = vec![KANI.command, "-p", krate, "--harness", harness, "--exact", "--output-format", "terse"];
        args.extend(["-Z", "unstable-options", "--harness-timeout", TIMEOUT]);
        // Compiled first with the same arguments (the crate is compiled for each proof), in a scope of its own: the
        // peak of a scope is from its start, and would be the compiling's.
        let mut codegen = cargo(None, &args);
        codegen.arg("--only-codegen");
        let compiled = log.run(capped(codegen, cap, false))?.success();
        let harness_log = run.dir.join(format!("kani-{}.log", harness.replace("::", ".")));
        let measured = Log::create(&harness_log, false)?;
        let started = Instant::now();
        let status = if !compiled {
            None
        } else if no_scope() {
            Some(measured.run(cargo(None, &args))?)
        } else {
            // This program in the scope runs the proof and says, once it ends, the peak of the scope.
            let mut inner = Command::new(this());
            inner.arg("__kani-harness").arg("cargo").args(&args);
            inner.env_remove("RUSTUP_TOOLCHAIN").env_remove("RUSTUP_TOOLCHAIN_SOURCE");
            Some(measured.run(capped(inner, cap, true))?)
        };
        let seconds = started.elapsed().as_secs();
        let text = fs::read_to_string(&harness_log).unwrap_or_default();
        let peak = text.lines().rev().find_map(|line| line.strip_prefix("memory.peak: ")?.parse::<u64>().ok());
        let verdict = match status {
            None => "failed: does not compile",
            Some(status) if status.success() && text.contains("VERIFICATION:- SUCCESSFUL") => "succeeded",
            Some(_) if text.contains("timed out") => "failed: timed out",
            Some(status)
                if status.signal().is_some()
                    || peak.is_some_and(|peak| peak >= cap)
                    || text.contains("killed by signal") =>
            {
                "failed: killed at the cap of memory"
            }
            Some(_) => "failed",
        };
        if verdict != "succeeded" {
            failed.push(harness.as_str());
        }
        let memory = peak.map_or("unknown".to_owned(), size);
        let line = format!("{harness}: {verdict}, {seconds}s, {memory}");
        log.say(&format!("kani: {line}"));
        table += &line;
        table.push('\n');
    }
    let _ = writeln!(table, "cap of memory: {}; the time and memory of the proof alone, compiled before", size(cap));
    fs::write(run.dir.join("harnesses.txt"), table)?;
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} of {} proofs failed: {}", failed.len(), harnesses.len(), failed.join(", ")).into())
    }
}

/// The name of the package `file` (relative to the root) is in.
fn package(file: &Path) -> Result<String> {
    for dir in file.ancestors().skip(1) {
        let Ok(manifest) = fs::read_to_string(root().join(dir).join("Cargo.toml")) else { continue };
        let mut in_package = false;
        for line in manifest.lines().map(str::trim) {
            if line.starts_with('[') {
                in_package = line == "[package]";
            } else if in_package && let Some(name) = line.strip_prefix("name") {
                return Ok(name.trim_start().trim_start_matches('=').trim().trim_matches('"').to_owned());
            }
        }
    }
    Err(format!("no package has {}", file.display()).into())
}

/// `cargo xtask __kani-harness <command>…`, which `run` runs in the scope of a proof: runs the command and, once it
/// ends, writes the peak of memory of the scope (every process in it), which Linux keeps from 5.19.
pub fn harness(command: &[String]) -> Result<i32> {
    let (program, args) = command.split_first().ok_or("__kani-harness <command>…")?;
    let status = Command::new(program).args(args).status()?;
    if let Some(signal) = status.signal() {
        println!("killed by signal {signal}");
    }
    // Not there before Linux 5.19: the peak is then unknown, the proof's result the same.
    if let Ok(peak) = cgroup().and_then(|dir| Ok(fs::read_to_string(dir.join("memory.peak"))?)) {
        println!("memory.peak: {}", peak.trim());
    }
    Ok(status.code().unwrap_or(1))
}
