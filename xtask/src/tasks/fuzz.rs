//! The fuzz targets, side by side in one scope whose cap is on them together.

use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;

use super::{Kind, MEMORY, Plan, succeed};
use crate::pins::{DENY, FUZZ, NIGHTLY};
use crate::record::Run;
use crate::run::{Log, Result, bytes, capped, cargo, memory, root, size, this};

/// For each fuzz target: the memory it may take, and how long one input may.
const FLAGS: [&str; 2] = ["-rss_limit_mb=1024", "-timeout=10"];
/// What a fuzz target is given in the scope: its `-rss_limit_mb`, and room above it.
const TARGET_MEMORY: u64 = 1280 << 20;
pub const SECONDS: u64 = 3600;

pub fn plan(args: &[String], recorded: bool) -> Result<std::result::Result<Plan, String>> {
    let (seconds, only) = match args.split_first().and_then(|(first, rest)| Some((first.parse::<u64>().ok()?, rest))) {
        // libFuzzer takes 0 seconds for no limit.
        Some((0, _)) => return Ok(Err("fuzzing for 0 seconds would fuzz without end".into())),
        Some((seconds, rest)) => (seconds, rest),
        None => (SECONDS, args),
    };
    if recorded && (seconds != SECONDS || !only.is_empty()) {
        return Ok(Err(format!(
            "fuzzing other than every target for {SECONDS}s may be run with --no-record only: recorded, it would \
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
    let needed = TARGET_MEMORY * jobs as u64;
    let run = match env::var("RUST_XTASK_FUZZ_MEMORY") {
        Ok(value) => match bytes(&value) {
            Some(run) if run < needed => {
                return Ok(Err(format!(
                    "RUST_XTASK_FUZZ_MEMORY={value} is below the {} {jobs} targets at once need ({} each): the scope \
                     would kill them all before libFuzzer stops one",
                    size(needed),
                    size(TARGET_MEMORY)
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

pub fn run(seconds: u64, targets: &[String], jobs: usize, (build, cap): (u64, u64), run: &Run) -> Result<()> {
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

/// `cargo xtask __fuzz-targets <seconds> <jobs> <dir> <target>…`, which `run` runs inside the scope: `jobs` targets
/// at once, each a process (no -jobs, whose fuzz-N.log would go where cargo fuzz runs), its output in fuzz-<target>.log
/// in `dir`.
pub fn targets(args: &[String]) -> Result<bool> {
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
                        command.arg(format!("-max_total_time={seconds}")).args(FLAGS);
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
