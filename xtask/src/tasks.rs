//! The tasks of the audit: what each checks before it runs, and how it runs.

use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Instant;

use crate::json::{self, Value};
use crate::pins::{DENY, FUZZ, GEIGER, KANI, LLVM_COV, NIGHTLY, VET};
use crate::record::Run;
use crate::run::{Log, Result, bytes, capped, cargo, cgroup, memory, no_scope, root, size, this};

/// The crate the tests, proofs and coverage are of; the binding is a cdylib on the C API of Python, which Miri cannot
/// run and which the Python tests cover.
const CORE: &str = "layeredconf";
/// The tests write files and read the environment.
const MIRI_FLAGS: &str = "-Zmiri-disable-isolation";
const KANI_TIMEOUT: &str = "1800s";
/// For each fuzz target: the memory it may take, and how long one input may.
const FUZZ_FLAGS: [&str; 2] = ["-rss_limit_mb=1024", "-timeout=10"];
/// What a fuzz target is given in the scope: its `-rss_limit_mb`, and room above it.
const FUZZ_TARGET_MEMORY: u64 = 1280 << 20;
pub const FUZZ_SECONDS: u64 = 3600;
/// The cap of a heavy task without `RUST_XTASK_MEMORY`.
const MEMORY: u64 = 4 << 30;
/// The cap of a proof without `RUST_XTASK_MEMORY`: the larger of 4G and 1.5 times the largest peak of memory.peak of a
/// proof (scope and all), measured with `RUST_XTASK_MEMORY=16G cargo xtask kani --no-record`; to be measured again
/// when the proofs change. The largest was 10.8 GiB (`parse_int_is_exact_past_u64`, 2026-11-01), 16.2 GiB times 1.5.
const KANI_MEMORY: u64 = 17 << 30;

/// Whether `task` runs under a cap of memory.
pub fn heavy(task: &str) -> bool {
    !matches!(task, "deny" | "vet")
}

/// A task ready to run, its arguments read and checked.
pub struct Plan {
    pub task: &'static str,
    /// The cap of memory it runs under, in bytes; the larger one for the fuzzing, which has two.
    pub cap: u64,
    kind: Kind,
}

enum Kind {
    Plain,
    Kani { only: Vec<String> },
    Fuzz { seconds: u64, targets: Vec<String>, jobs: usize, build: u64, run: u64 },
}

impl Plan {
    /// Its arguments, as run.txt keeps them.
    pub fn arguments(&self) -> String {
        match &self.kind {
            Kind::Fuzz { seconds, .. } => format!("seconds={seconds}"),
            _ => String::new(),
        }
    }
}

/// The arguments of `task` run as recorded runs are, by default.
pub fn arguments(task: &str) -> String {
    if task == "fuzz" { format!("seconds={FUZZ_SECONDS}") } else { String::new() }
}

/// Reads the arguments of `task` and makes the checks of its own before it runs: Err with why it may not.
pub fn plan(task: &'static str, args: &[String], recorded: bool) -> Result<std::result::Result<Plan, String>> {
    let plain = |cap| Ok(Ok(Plan { task, cap, kind: Kind::Plain }));
    if task != "kani" && task != "fuzz" && !args.is_empty() {
        return Ok(Err(format!("{task} takes no arguments")));
    }
    match task {
        "deny" | "vet" => plain(0),
        "kani" => {
            if !args.is_empty() && recorded {
                return Ok(Err("some proofs only may be run with --no-record: they are not the results of all".into()));
            }
            Ok(Ok(Plan { task, cap: memory(KANI_MEMORY)?, kind: Kind::Kani { only: args.to_vec() } }))
        }
        "fuzz" => plan_fuzz(args, recorded),
        _ => plain(memory(MEMORY)?),
    }
}

fn plan_fuzz(args: &[String], recorded: bool) -> Result<std::result::Result<Plan, String>> {
    let (seconds, only) = match args.split_first().and_then(|(first, rest)| Some((first.parse::<u64>().ok()?, rest))) {
        // libFuzzer takes 0 seconds for no limit.
        Some((0, _)) => return Ok(Err("fuzzing for 0 seconds would fuzz without end".into())),
        Some((seconds, rest)) => (seconds, rest),
        None => (FUZZ_SECONDS, args),
    };
    if recorded && (seconds != FUZZ_SECONDS || !only.is_empty()) {
        return Ok(Err(format!(
            "fuzzing other than every target for {FUZZ_SECONDS}s may be run with --no-record only: recorded, it would \
             take the place of the full run"
        )));
    }
    // Only fuzz/Cargo.toml is read: nothing is built.
    let listed = cargo(Some(NIGHTLY), &[FUZZ.command, "list", "--fuzz-dir", "fuzz"]).output()?;
    let every: Vec<String> = String::from_utf8_lossy(&listed.stdout).lines().map(str::to_owned).collect();
    if !listed.status.success() || every.is_empty() {
        return Ok(Err(format!("cargo fuzz list found no target ({})", listed.status)));
    }
    if let Some(unknown) = only.iter().find(|name| !every.contains(name)) {
        return Ok(Err(format!("no fuzz target {unknown}; there are {}", every.join(", "))));
    }
    let targets: Vec<String> = if only.is_empty() { every } else { only.to_vec() };
    let jobs = match env::var("RUST_XTASK_JOBS") {
        Ok(value) => match value.parse::<usize>() {
            Ok(jobs @ 1..) => jobs,
            _ => return Ok(Err(format!("RUST_XTASK_JOBS={value} is no number of targets at once"))),
        },
        Err(_) => targets.len(),
    }
    .min(targets.len());
    let needed = FUZZ_TARGET_MEMORY * jobs as u64;
    let run = match env::var("RUST_XTASK_FUZZ_MEMORY") {
        Ok(value) => match bytes(&value) {
            Some(run) if run < needed => {
                return Ok(Err(format!(
                    "RUST_XTASK_FUZZ_MEMORY={value} is below the {} {jobs} targets at once need ({} each): the scope \
                     would kill them all before libFuzzer stops one",
                    size(needed),
                    size(FUZZ_TARGET_MEMORY)
                )));
            }
            Some(run) => run,
            None => return Ok(Err(format!("RUST_XTASK_FUZZ_MEMORY={value} is no size (4G, 1536M)"))),
        },
        Err(_) => needed,
    };
    // Before anything is built: libFuzzer is C++ built from source, and cargo fuzz build would rewrite a fuzz/Cargo.lock
    // out of date, which --locked refuses here.
    warn_if_locks_differ();
    println!("fuzz: cargo deny of the dependencies of the fuzz targets, before they are built");
    let checked = cargo(
        None,
        &[DENY.command, "--locked", "--manifest-path", "fuzz/Cargo.toml", "check", "advisories", "sources"],
    )
    .status()?;
    if !checked.success() {
        return Ok(Err("the dependencies of the fuzz targets do not pass cargo deny (advisories, sources)".into()));
    }
    let build = memory(MEMORY)?;
    Ok(Ok(Plan { task: "fuzz", cap: build.max(run), kind: Kind::Fuzz { seconds, targets, jobs, build, run } }))
}

/// Runs the task `plan` is of; Err with why it failed.
pub fn run(plan: &Plan, run: &Run) -> Result<()> {
    let log = &run.log;
    match (plan.task, &plan.kind) {
        ("deny", _) => deny(log),
        ("vet", _) => {
            // --locked: without it cargo vet writes supply-chain/imports.lock anew.
            succeed(log.run(cargo(None, &[VET.command, "--locked"]))?.success(), "cargo vet found crates not vetted")
        }
        ("geiger", _) => geiger(plan, run),
        ("miri", _) => {
            let mut command = cargo(Some(NIGHTLY), &["miri", "test", "--locked", "-p", CORE]);
            command.env("MIRIFLAGS", MIRI_FLAGS);
            succeed(log.run(capped(command, plan.cap, false))?.success(), "a test failed under Miri")
        }
        ("coverage", _) => coverage(plan, run),
        (_, Kind::Kani { only }) => kani(plan.cap, only, run),
        (_, Kind::Fuzz { seconds, targets, jobs, build, run: cap }) => {
            fuzz(*seconds, targets, *jobs, (*build, *cap), run)
        }
        _ => Err(format!("no task {}", plan.task).into()),
    }
}

fn succeed(ok: bool, otherwise: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(otherwise.into()) }
}

fn deny(log: &Log) -> Result<()> {
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

fn geiger(plan: &Plan, run: &Run) -> Result<()> {
    // cargo geiger runs offline, on the sources of the dependencies, which a library does not vendor.
    succeed(run.log.run(cargo(None, &["fetch", "--locked"]))?.success(), "cargo fetch failed")?;
    // From the binding, whose dependencies are all those of the library and more.
    let mut command = cargo(None, &[GEIGER.command, "--locked", "--offline"]);
    command.current_dir(root().join("crates/layeredconf-python"));
    let table = run.dir.join("geiger.txt");
    run.log.run_into(capped(command, plan.cap, false), &table)?;
    // cargo geiger 0.14 ends in failure for files it did not scan (the README the crates include): it ran if it wrote
    // its table. A count to read, not a check.
    succeed(fs::read_to_string(&table)?.contains("Metric output format"), "cargo geiger wrote no table")
}

fn coverage(plan: &Plan, run: &Run) -> Result<()> {
    let log = &run.log;
    // --no-report implies --no-clean: the profiles of earlier runs, of code since changed, would be merged in.
    succeed(log.run(cargo(Some(NIGHTLY), &[LLVM_COV.command, "clean", "--workspace"]))?.success(), "cleaning failed")?;
    let test = cargo(Some(NIGHTLY), &[LLVM_COV.command, "--branch", "--no-report", "--locked", "-p", CORE]);
    succeed(log.run(capped(test, plan.cap, false))?.success(), "a test failed")?;
    let report = cargo(Some(NIGHTLY), &[LLVM_COV.command, "report", "--branch", "-p", CORE]);
    succeed(log.run_into(report, &run.dir.join("summary.txt"))?.success(), "cargo llvm-cov report failed")?;
    // Hundreds of files, nearly all changed every time: kept in target/ to look at, not committed.
    let html = cargo(Some(NIGHTLY), &[LLVM_COV.command, "report", "--branch", "-p", CORE, "--html"]);
    succeed(log.run(html)?.success(), "cargo llvm-cov report --html failed")?;
    log.say("coverage: the lines and branches not run are marked in target/llvm-cov/html/index.html");
    Ok(())
}

fn kani(cap: u64, only: &[String], run: &Run) -> Result<()> {
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
        args.extend(["-Z", "unstable-options", "--harness-timeout", KANI_TIMEOUT]);
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

/// `cargo xtask __kani-harness <command>…`, which `kani` runs in the scope of a proof: runs the command and, once it
/// ends, writes the peak of memory of the scope (every process in it), which Linux keeps from 5.19.
pub fn kani_harness(command: &[String]) -> Result<i32> {
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

fn fuzz(seconds: u64, targets: &[String], jobs: usize, (build, cap): (u64, u64), run: &Run) -> Result<()> {
    let log = &run.log;
    // Built alone first: compiling libFuzzer's C++ takes what it takes, however many targets there are.
    let compile = cargo(Some(NIGHTLY), &[FUZZ.command, "build", "--fuzz-dir", "fuzz"]);
    succeed(log.run(capped(compile, build, false))?.success(), "cargo fuzz build failed")?;
    let before = artifacts();
    log.say(&format!(
        "fuzz: {} for {seconds}s each, {jobs} at once, capped at {} together",
        targets.join(", "),
        size(cap)
    ));
    // Every target in one scope, so that the cap is on the fuzzing as a whole: this program again, inside it.
    let mut inner = Command::new(this());
    inner.arg("__fuzz-targets").arg(seconds.to_string()).arg(jobs.to_string()).arg(&run.dir).args(targets);
    log.run(capped(inner, cap, false))?;
    let found: Vec<String> = artifacts().difference(&before).cloned().collect();
    let mut table = String::new();
    let mut failed = Vec::new();
    for target in targets {
        let text = fs::read_to_string(run.dir.join(format!("fuzz-{target}.log"))).unwrap_or_default();
        let status = text.lines().last().unwrap_or("[not run]");
        let runs: u64 =
            text.lines().filter_map(|line| line.strip_prefix("Done ")?.split(' ').next()?.parse::<u64>().ok()).sum();
        let crashes: Vec<&String> = found
            .iter()
            .filter(|path| path.starts_with(&format!("{target}/")) && !path.contains("/slow-unit-"))
            .collect();
        let ok = status == "[exit status: 0]" && crashes.is_empty();
        let mut line = format!("{target}: {}, {runs} runs ({status})", if ok { "succeeded" } else { "failed" });
        for crash in crashes {
            let _ = write!(line, "; fuzz/artifacts/{crash}");
        }
        if !ok {
            failed.push(target.as_str());
        }
        log.say(&format!("fuzz: {line}"));
        table += &line;
        table.push('\n');
    }
    for slow in found.iter().filter(|path| path.contains("/slow-unit-")) {
        let line = format!("slow input fuzz/artifacts/{slow} (a busy machine makes them too)");
        log.say(&format!("fuzz: {line}"));
        table += &line;
        table.push('\n');
    }
    fs::write(run.dir.join("targets.txt"), table)?;
    if failed.is_empty() { Ok(()) } else { Err(format!("fuzz targets failed: {}", failed.join(", ")).into()) }
}

/// `cargo xtask __fuzz-targets <seconds> <jobs> <dir> <target>…`, which `fuzz` runs inside the scope: `jobs` targets
/// at once, each a process (no -jobs, whose fuzz-N.log would go where cargo fuzz runs), its output in fuzz-<target>.log
/// in `dir`.
pub fn fuzz_targets(args: &[String]) -> Result<bool> {
    let [seconds, jobs, dir, targets @ ..] = args else {
        return Err("__fuzz-targets <seconds> <jobs> <dir> <target>…".into());
    };
    let jobs: usize = jobs.parse()?;
    let mut ok = true;
    for batch in targets.chunks(jobs.max(1)) {
        let statuses = thread::scope(|scope| {
            let running: Vec<_> = batch
                .iter()
                .map(|target| {
                    scope.spawn(move || -> Result<bool> {
                        println!("fuzz: {target} started");
                        let log = Log::create(&Path::new(dir).join(format!("fuzz-{target}.log")), false)?;
                        let mut command =
                            cargo(Some(NIGHTLY), &[FUZZ.command, "run", "--fuzz-dir", "fuzz", target, "--"]);
                        command.arg(format!("-max_total_time={seconds}")).args(FUZZ_FLAGS);
                        let status = log.run(command)?;
                        println!("fuzz: {target} ended: {status}");
                        Ok(status.success())
                    })
                })
                .collect();
            running.into_iter().map(|handle| handle.join().is_ok_and(|ran| ran.is_ok_and(|ok| ok))).collect::<Vec<_>>()
        });
        ok &= statuses.into_iter().all(|ok| ok);
    }
    Ok(ok)
}

/// The files under fuzz/artifacts/, as "target/name".
fn artifacts() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for target in fs::read_dir(root().join("fuzz/artifacts")).into_iter().flatten().flatten() {
        for file in fs::read_dir(target.path()).into_iter().flatten().flatten() {
            found.insert(format!("{}/{}", target.file_name().to_string_lossy(), file.file_name().to_string_lossy()));
        }
    }
    found
}

/// The fuzz workspace has a lock of its own, started from the main one so that the crates both have are those a user
/// builds; says so when they differ (`--locked` does not see it: the fuzz lock may still fit its manifest).
fn warn_if_locks_differ() {
    let packages = |path: &str| -> BTreeSet<(String, String)> {
        let text = fs::read_to_string(root().join(path)).unwrap_or_default();
        let mut found = BTreeSet::new();
        let mut name = None;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("name = ") {
                name = Some(value.trim_matches('"').to_owned());
            } else if let (Some(value), Some(named)) = (line.strip_prefix("version = "), name.take()) {
                found.insert((named, value.trim_matches('"').to_owned()));
            }
        }
        found
    };
    let (main, fuzz) = (packages("Cargo.lock"), packages("fuzz/Cargo.lock"));
    let differ: Vec<String> = fuzz
        .iter()
        .filter(|(name, version)| {
            main.iter().any(|(other, at)| other == name && at != version)
                && !main.contains(&(name.clone(), version.clone()))
        })
        .map(|(name, version)| format!("{name} {version}"))
        .collect();
    if !differ.is_empty() {
        println!(
            "fuzz: fuzz/Cargo.lock has {} at other versions than Cargo.lock; to align it: \
             cp Cargo.lock fuzz/ && (cd fuzz && cargo metadata --offline > /dev/null)",
            differ.join(", ")
        );
    }
}
