//! The audit of layeredconf, apart from its build and its tests: `cargo xtask <task>`.
//!
//! The tools and the nightly Rust the audit needs are pinned in tools.rs, so that every machine runs the same versions,
//! and `cargo xtask setup` installs them. The heavy tasks run one at a time, each in a systemd user scope whose memory
//! is capped, so that one going too far is killed alone and not the machine. The logs, and a line a task in
//! summary.txt, go to target/audit/.

mod json;
mod outdated;
mod run;
mod tools;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::Instant;

use run::{Log, Result, audit_dir, capped, cargo, record, root, timed};
use tools::NIGHTLY;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  setup [task…]             install the pinned tools the tasks need (of every task by default)
  deny                      cargo deny: advisories, licenses, duplicates, sources
  vet                       cargo vet: every dependency audited or exempted
  geiger                    cargo geiger: the unsafe code of the dependencies
  miri                      the tests of layeredconf under Miri
  coverage                  branch coverage of the tests of layeredconf
  kani [harness…]           the Kani proofs, one at a time (every one by default)
  fuzz [seconds] [target…]  the fuzz targets side by side (every one, for an hour, by default)
  all                       each of deny, vet, geiger, miri, coverage, kani and fuzz in turn
  outdated [--vulns]        the pinned versions due an upgrade, with an advisory or a month behind a release

environment:
  RUST_XTASK_MEMORY          cap of memory of a heavy task (16G for a proof, 6G Miri, 8G coverage, 3G a fuzz target)
  RUST_XTASK_NO_SCOPE        run the heavy tasks without a systemd scope
  RUST_XTASK_VULNS_EVERY     days between the checks for advisories the other tasks make (1)
  RUST_XTASK_RELEASES_EVERY  days between their checks for releases (7)

Logs and summary.txt are in target/audit/.";

/// The crate the tests, proofs and coverage are of; the binding is a cdylib on the C API of Python, which Miri cannot
/// run and which the Python tests cover.
const CORE: &str = "layeredconf";
/// The tests write files and read the environment.
const MIRI_FLAGS: &str = "-Zmiri-disable-isolation";
const KANI_TIMEOUT: &str = "1800s";
/// For each fuzz target: workers side by side, the memory of each, and how long one input may take.
const FUZZ_FLAGS: [&str; 4] = ["-jobs=2", "-workers=2", "-rss_limit_mb=1024", "-timeout=10"];
const FUZZ_SECONDS: u64 = 3600;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((task, rest)) = args.split_first() else {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let ran = match task.as_str() {
        "setup" => tools::setup(rest).map(|()| true),
        "outdated" => match rest {
            [] => outdated::outdated(false),
            [vulns] if vulns == "--vulns" => outdated::outdated(true),
            _ => Err("usage: cargo xtask outdated [--vulns]".into()),
        }
        .map(|current| {
            if !current {
                // Nobody to ask: a failure CI shows.
                std::process::exit(2);
            }
            true
        }),
        "deny" | "vet" | "geiger" | "miri" | "coverage" | "kani" | "fuzz" | "all" => {
            outdated::notice();
            if task == "all" { Ok(all()) } else { audit(task, rest) }
        }
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(true)
        }
        _ => Err(format!("no task {task}\n\n{USAGE}").into()),
    };
    match ran {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("xtask {task}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs a task of the audit if what it needs is installed; whether it passed.
fn audit(task: &str, args: &[String]) -> Result<bool> {
    let missing = tools::missing(task);
    if !missing.is_empty() {
        return Err(format!("{}; cargo xtask setup {task} installs it", missing.join("; ")).into());
    }
    match task {
        "deny" => deny(),
        "vet" => vet(),
        "geiger" => geiger(),
        "miri" => miri(),
        "coverage" => coverage(),
        "kani" => kani(args),
        "fuzz" => fuzz(args),
        _ => unreachable!("main gives only tasks"),
    }
}

/// Every task in turn, the next after one that fails too.
fn all() -> bool {
    let mut failed = Vec::new();
    for task in ["deny", "vet", "geiger", "miri", "coverage", "kani", "fuzz"] {
        println!("== {task}");
        match audit(task, &[]) {
            Ok(true) => {}
            Ok(false) => failed.push(task),
            Err(error) => {
                record(task, &format!("not run: {error}"));
                failed.push(task);
            }
        }
    }
    record("all", &if failed.is_empty() { "passed".to_owned() } else { format!("failed: {}", failed.join(", ")) });
    failed.is_empty()
}

/// Records how a task of one command went.
fn outcome(task: &str, passed: bool, log: &Log) -> bool {
    let verdict = if passed { "passed" } else { "FAILED" };
    record(task, &format!("{verdict} ({})", log.path().display()));
    passed
}

fn deny() -> Result<bool> {
    let log = Log::create(audit_dir()?.join("deny.log"))?;
    let main = log.run(cargo(None, &["deny", "--locked", "check"]), true)?;
    // The fuzz targets are not built by a user of the library, but are built and run here; their dependencies (libFuzzer
    // is C++ built from source) are checked before.
    let fuzz = log.run(
        cargo(None, &["deny", "--locked", "--manifest-path", "fuzz/Cargo.toml", "check", "advisories", "sources"]),
        true,
    )?;
    Ok(outcome("deny", main.success() && fuzz.success(), &log))
}

fn vet() -> Result<bool> {
    let log = Log::create(audit_dir()?.join("vet.log"))?;
    let status = log.run(cargo(None, &["vet", "--locked"]), true)?;
    Ok(outcome("vet", status.success(), &log))
}

fn geiger() -> Result<bool> {
    let log = Log::create(audit_dir()?.join("geiger.log"))?;
    // From the binding, whose dependencies are all those of the library and more.
    let mut command = cargo(None, &["geiger", "--locked", "--offline"]);
    command.current_dir(root().join("crates/layeredconf-python"));
    log.run(command, false)?;
    // cargo geiger 0.14 ends in failure for files it did not scan (the README the crates include): it passed if it
    // wrote its table.
    let passed = log.text().contains("Metric output format");
    Ok(outcome("geiger", passed, &log))
}

fn miri() -> Result<bool> {
    let log = Log::create(audit_dir()?.join("miri.log"))?;
    let mut command = cargo(Some(NIGHTLY), &["miri", "test", "--locked", "-p", CORE]);
    command.env("MIRIFLAGS", MIRI_FLAGS);
    let status = log.run(capped(command, "6G"), true)?;
    Ok(outcome("miri", status.success(), &log))
}

fn coverage() -> Result<bool> {
    let dir = audit_dir()?.join("coverage");
    let log = Log::create(audit_dir()?.join("coverage.log"))?;
    let test = cargo(Some(NIGHTLY), &["llvm-cov", "--branch", "--locked", "-p", CORE, "--summary-only"]);
    let mut passed = log.run(capped(test, "8G"), true)?.success();
    if passed {
        let mut html = cargo(Some(NIGHTLY), &["llvm-cov", "report", "--branch", "-p", CORE, "--html", "--output-dir"]);
        html.arg(&dir);
        passed = log.run(html, false)?.success();
        println!("coverage: the lines and branches not run are marked in {}", dir.join("html/index.html").display());
    }
    Ok(outcome("coverage", passed, &log))
}

/// The harnesses of the proofs, from the source.
fn harnesses() -> Result<Vec<String>> {
    let source = fs::read_to_string(root().join("crates/layeredconf/src/proofs.rs"))?;
    let mut harnesses = Vec::new();
    let mut proof = false;
    for line in source.lines().map(str::trim) {
        if line == "#[kani::proof]" {
            proof = true;
        } else if let Some(name) = line.strip_prefix("fn ").filter(|_| proof) {
            harnesses.push(name.split('(').next().unwrap_or(name).to_owned());
            proof = false;
        }
    }
    Ok(harnesses)
}

fn kani(only: &[String]) -> Result<bool> {
    let every = harnesses()?;
    if let Some(unknown) = only.iter().find(|name| !every.contains(name)) {
        return Err(format!("no harness {unknown}; there are {}", every.join(", ")).into());
    }
    let dir = audit_dir()?.join("kani");
    fs::create_dir_all(&dir)?;
    let mut failed = 0;
    let chosen: Vec<&String> = every.iter().filter(|name| only.is_empty() || only.contains(name)).collect();
    for harness in &chosen {
        let log = Log::create(dir.join(format!("{harness}.log")))?;
        let full = format!("proofs::{harness}");
        let mut command = cargo(None, &["kani", "-p", CORE, "--harness", &full, "--exact", "--output-format", "terse"]);
        command.args(["-Z", "unstable-options", "--harness-timeout", KANI_TIMEOUT]);
        let started = Instant::now();
        let status = log.run(capped(timed(command), "16G"), false)?;
        let text = log.text();
        let verdict = if status.success() && text.contains("VERIFICATION:- SUCCESSFUL") {
            "proved"
        } else if text.contains("timed out") {
            "TIMED OUT"
        } else if text.contains("Command terminated by signal") || status.code().is_none() {
            "KILLED (past the cap of memory?)"
        } else {
            "FAILED"
        };
        failed += usize::from(verdict != "proved");
        let memory = run::peak_memory(&text).map(|peak| format!(", {peak}")).unwrap_or_default();
        record("kani", &format!("{full}: {verdict} in {}s{memory}", started.elapsed().as_secs()));
    }
    record("kani", &format!("{} of {} proved ({})", chosen.len() - failed, chosen.len(), dir.display()));
    Ok(failed == 0)
}

fn fuzz(args: &[String]) -> Result<bool> {
    let (seconds, only) = match args.split_first() {
        Some((seconds, rest)) if seconds.parse::<u64>().is_ok() => (seconds.parse::<u64>()?, rest),
        _ => (FUZZ_SECONDS, args),
    };
    let fuzz_dir = root().join("fuzz");
    let fuzz_dir_arg = fuzz_dir.to_string_lossy().into_owned();
    warn_if_locks_differ(&root().join("Cargo.lock"), &fuzz_dir.join("Cargo.lock"));
    let dir = audit_dir()?.join("fuzz");
    fs::create_dir_all(&dir)?;
    let build = Log::create(dir.join("build.log"))?;
    if !build.run(cargo(Some(NIGHTLY), &["fuzz", "build", "--fuzz-dir", &fuzz_dir_arg]), true)?.success() {
        return Ok(outcome("fuzz", false, &build));
    }
    let listed = cargo(Some(NIGHTLY), &["fuzz", "list", "--fuzz-dir", &fuzz_dir_arg]).output()?;
    let every: Vec<String> = String::from_utf8_lossy(&listed.stdout).lines().map(str::to_owned).collect();
    if let Some(unknown) = only.iter().find(|name| !every.contains(name)) {
        return Err(format!("no fuzz target {unknown}; there are {}", every.join(", ")).into());
    }
    let targets: Vec<&String> = every.iter().filter(|name| only.is_empty() || only.contains(name)).collect();
    let before = artifacts(&fuzz_dir);
    println!(
        "fuzz: {} for {seconds}s each, side by side",
        targets.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(", ")
    );
    let statuses = thread::scope(|scope| {
        let running: Vec<_> = targets
            .iter()
            .map(|target| {
                // A directory of its own: each worker writes fuzz-N.log where it runs.
                let work = dir.join(target);
                let fuzz_dir_arg = &fuzz_dir_arg;
                scope.spawn(move || -> Result<_> {
                    fs::create_dir_all(&work)?;
                    for old in fs::read_dir(&work)?
                        .flatten()
                        .filter(|entry| entry.file_name().to_string_lossy().starts_with("fuzz-"))
                    {
                        fs::remove_file(old.path())?;
                    }
                    let log = Log::create(work.join("main.log"))?;
                    let mut command = cargo(Some(NIGHTLY), &["fuzz", "run", "--fuzz-dir", fuzz_dir_arg, target, "--"]);
                    command.arg(format!("-max_total_time={seconds}")).args(FUZZ_FLAGS).current_dir(&work);
                    log.run(capped(command, "3G"), false)
                })
            })
            .collect();
        running
            .into_iter()
            .map(|handle| handle.join().unwrap_or_else(|_| Err("the thread panicked".into())))
            .collect::<Vec<_>>()
    });
    let found: Vec<String> = artifacts(&fuzz_dir).difference(&before).cloned().collect();
    let mut passed = true;
    for (target, status) in targets.iter().zip(statuses) {
        let runs = runs(&dir.join(target));
        let failures: Vec<&String> = found
            .iter()
            .filter(|path| path.starts_with(&format!("{target}/")) && !path.contains("/slow-unit-"))
            .collect();
        let ok = status.as_ref().is_ok_and(std::process::ExitStatus::success) && failures.is_empty();
        passed &= ok;
        let status = match status {
            Ok(status) => status.to_string(),
            Err(error) => error.to_string(),
        };
        let mut line = format!("{target}: {} ({runs} runs, {status})", if ok { "passed" } else { "FAILED" });
        for failure in failures {
            line += "; fuzz/artifacts/";
            line += failure;
        }
        record("fuzz", &line);
    }
    for slow in found.iter().filter(|path| path.contains("/slow-unit-")) {
        record("fuzz", &format!("slow input fuzz/artifacts/{slow} (a busy machine makes them too)"));
    }
    Ok(passed)
}

/// The files under fuzz/artifacts/, as "target/name".
fn artifacts(fuzz_dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for target in fs::read_dir(fuzz_dir.join("artifacts")).into_iter().flatten().flatten() {
        for file in fs::read_dir(target.path()).into_iter().flatten().flatten() {
            found.insert(format!("{}/{}", target.file_name().to_string_lossy(), file.file_name().to_string_lossy()));
        }
    }
    found
}

/// The inputs the workers of a target ran, from their "Done N runs" lines.
fn runs(work: &Path) -> u64 {
    let logs = fs::read_dir(work)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("fuzz-"));
    logs.filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| line.strip_prefix("Done ")?.split(' ').next()?.parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .sum()
}

/// The fuzz workspace has a lock of its own, started from the main one so that the crates both have are those a user
/// builds; says so when they differ.
fn warn_if_locks_differ(main: &Path, fuzz: &Path) {
    let packages = |path: &Path| -> BTreeSet<(String, String)> {
        let text = fs::read_to_string(path).unwrap_or_default();
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
    let (main, fuzz) = (packages(main), packages(fuzz));
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
