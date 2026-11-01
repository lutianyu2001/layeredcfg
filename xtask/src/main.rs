//! The audit of layeredconf, apart from its build and its tests: `cargo xtask <task>`.
//!
//! The tools and the nightly Rust the audit needs are pinned in pins.rs, so that every machine runs the same versions,
//! and `cargo xtask setup` installs them. The heavy tasks run in a systemd user scope whose memory is capped, so that
//! one going too far is killed alone and not the machine; one task at a time unless asked otherwise. The latest run of
//! each task is kept in audit/<task>/, committed with the code (see record.rs).

mod json;
mod lock;
mod outdated;
mod pins;
mod record;
mod run;
mod setup;
mod tasks;

use std::env;
use std::process::ExitCode;

use record::Run;
use run::{Result, has_scope, no_scope, root};
use setup::TASKS;

const USAGE: &str = "\
usage: cargo xtask <task> [--no-record] [--if-needed]

tasks, each recorded in audit/<task>/:
  deny                 cargo deny: advisories, licenses, duplicates, sources; of the fuzz targets too
  vet                  cargo vet: every dependency audited or exempted
  geiger               cargo geiger: the unsafe code of the dependencies, counted
  miri                 the tests of layeredconf under Miri
  coverage             branch coverage of the tests of layeredconf
  kani [harness…]      the Kani proofs, one at a time (some only with --no-record)
  fuzz [seconds] [target…]
                       the fuzz targets side by side, for an hour (otherwise only with --no-record)
  all                  deny, vet, geiger, miri, coverage, kani and fuzz in turn

  --no-record          run on any working tree, into target/audit/<task>/, audit/ left as it is
  --if-needed          skip a task whose recorded results still hold

and:
  status               the latest result of each task, and whether it still holds
  needed <task>        why the task is to run (exit 0) or not (exit 1); 2 for an error
  setup [task…]        install the pinned tools the tasks need (of every task by default)
  outdated [--vulns]   the pinned versions due an upgrade: with an advisory, or a month behind a release

environment:
  RUST_XTASK_MEMORY          cap of memory of a heavy task (4G; Kani's is in tasks.rs)
  RUST_XTASK_FUZZ_MEMORY     cap of the fuzz targets together (1280M for each at once)
  RUST_XTASK_JOBS            fuzz targets at once (every one)
  RUST_XTASK_PARALLEL=1      let tasks run side by side while their caps fit in the memory
  RUST_XTASK_NO_SCOPE=1      run the heavy tasks without a cap (in CI)
  RUST_XTASK_VULNS_EVERY     days between the checks for advisories the other tasks make (1)
  RUST_XTASK_RELEASES_EVERY  days between their checks for releases (7)";

fn main() -> ExitCode {
    if let Err(error) = env::set_current_dir(root()) {
        eprintln!("xtask: cannot go to {}: {error}", root().display());
        return ExitCode::FAILURE;
    }
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let ran: Result<u8> = match command.as_str() {
        "setup" => lock::audit(false).and_then(|_lock| setup::setup(rest)).map(|()| 0),
        "outdated" => outdated(rest),
        "status" => record::status(tasks::arguments).map(|()| 0),
        "needed" => match rest {
            [task] if TASKS.contains(&task.as_str()) => {
                record::tree().map(|tree| match record::holds(task, &tasks::arguments(task), &tree) {
                    Ok(why) => {
                        println!("{task}: not needed: {why}");
                        1
                    }
                    Err(why) => {
                        println!("{task}: needed: {why}");
                        0
                    }
                })
            }
            _ => Err(format!("usage: cargo xtask needed <task>, a task of {}", TASKS.join(", ")).into()),
        }
        .or_else(|error| {
            eprintln!("xtask needed: {error}");
            Ok(2)
        }),
        "__scope-probe" => run::scope_probe().map(|()| 0),
        "__kani-harness" => tasks::kani_harness(rest).map(|code| u8::try_from(code).unwrap_or(1)),
        "__fuzz-targets" => tasks::fuzz_targets(rest).map(|ok| u8::from(!ok)),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(0)
        }
        task if task == "all" || TASKS.contains(&task) => audit(task, rest),
        _ => Err(format!("no task {command}\n\n{USAGE}").into()),
    };
    match ran {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("xtask {command}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn outdated(args: &[String]) -> Result<u8> {
    let vulns = match args {
        [] => false,
        [flag] if flag == "--vulns" => true,
        _ => return Err("usage: cargo xtask outdated [--vulns]".into()),
    };
    let lock = lock::audit(false)?;
    let (code, upgraded) = outdated::outdated(vulns)?;
    if upgraded {
        // Let go first: setup takes the lock. Through cargo, so that xtask is built again with the versions now
        // pinned.
        drop(lock);
        println!("outdated: installing the versions now pinned");
        let status = run::cargo(None, &["xtask", "setup"]).status()?;
        if !status.success() {
            return Err(format!("cargo xtask setup failed: {status}").into());
        }
    }
    Ok(code)
}

/// A task of the audit, or all of them in turn: 0 if each succeeded or was skipped.
fn audit(task: &str, args: &[String]) -> Result<u8> {
    let recorded = !args.iter().any(|arg| arg == "--no-record");
    let if_needed = args.iter().any(|arg| arg == "--if-needed");
    let args: Vec<String> = args.iter().filter(|arg| *arg != "--no-record" && *arg != "--if-needed").cloned().collect();
    // A word misspelled would be taken for no option at all, and a run recorded that was not to be.
    if let Some(unknown) = args.iter().find(|arg| arg.starts_with('-')) {
        return Err(format!("no option {unknown}").into());
    }
    if if_needed && !recorded {
        return Err("--if-needed asks whether the recorded results hold, and --no-record records none".into());
    }
    if task == "all" && !args.is_empty() {
        return Err("all takes no arguments".into());
    }
    // Held to the end, all's every task under it.
    let _lock = lock::audit(lock::parallel())?;
    outdated::notice();
    if no_scope() {
        println!("xtask: RUST_XTASK_NO_SCOPE=1: the heavy tasks run without a cap of memory");
    }
    let chosen: Vec<&'static str> = TASKS.iter().copied().filter(|name| task == "all" || *name == task).collect();
    let mut ok = true;
    for task in chosen {
        ok &= attempt(task, &args, recorded, if_needed);
    }
    Ok(u8::from(!ok))
}

/// One task: skipped if its results hold and `if_needed`, refused if a check before it fails, run otherwise. Whether
/// it succeeded or was skipped; says which in a line.
fn attempt(task: &'static str, args: &[String], recorded: bool, if_needed: bool) -> bool {
    let say = |what: &str| println!("xtask: {task}: {what}");
    let refuse = |why: &str| {
        say(&format!("refused: {why}"));
        false
    };
    if if_needed {
        match record::tree().map(|tree| record::holds(task, &tasks::arguments(task), &tree)) {
            Ok(Ok(why)) => {
                say(&format!("skipped: {why}"));
                return true;
            }
            Ok(Err(_)) => {}
            Err(error) => return refuse(&format!("cannot tell whether its results hold: {error}")),
        }
    }
    let missing = setup::missing(task);
    if !missing.is_empty() {
        return refuse(&format!("{}; cargo xtask setup {task} installs it", missing.join("; ")));
    }
    match record::uncommitted() {
        Ok(Some(why)) if recorded => return refuse(&format!("{why}; commit them, or run with --no-record")),
        Ok(_) => {}
        Err(error) => return refuse(&error.to_string()),
    }
    match record::lock_tracked() {
        Ok(true) => {}
        Ok(false) => return refuse("Cargo.lock is not tracked by git, and the results tell by it what was built"),
        Err(error) => return refuse(&error.to_string()),
    }
    if tasks::heavy(task) && !no_scope() && !has_scope() {
        return refuse(
            "no usable memory-limited scope (systemd user scope with MemoryMax); RUST_XTASK_NO_SCOPE=1 runs without one",
        );
    }
    let plan = match tasks::plan(task, args, recorded) {
        Ok(Ok(plan)) => plan,
        Ok(Err(why)) => return refuse(&why),
        Err(error) => return refuse(&error.to_string()),
    };
    let _held = match lock::task(task, plan.cap) {
        Ok(Ok(held)) => held,
        Ok(Err(why)) => return refuse(&why),
        Err(error) => return refuse(&error.to_string()),
    };
    let run = match Run::start(task, plan.arguments(), recorded) {
        Ok(run) => run,
        Err(error) => return refuse(&format!("cannot start a run: {error}")),
    };
    let outcome = tasks::run(&plan, &run);
    match run.finish(&outcome) {
        Ok(line) => say(&line),
        Err(error) => {
            say(&format!("failed: cannot record the run: {error}"));
            return false;
        }
    }
    outcome.is_ok()
}
