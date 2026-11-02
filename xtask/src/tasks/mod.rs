//! The tasks of the audit: what each checks before it runs, and how it runs. What they share is here, each task's own
//! code in a module of its own, which only that task's results depend on (see `record::affects`).

mod coverage;
mod deny;
pub mod fuzz;
mod geiger;
pub mod kani;
mod miri;
mod vet;

use crate::record::Run;
use crate::run::{Result, memory};

/// The crate the tests, proofs and coverage are of; the binding is a cdylib on the C API of Python, which Miri cannot
/// run and which the Python tests cover.
const CORE: &str = "layeredconf";
/// The cap of a heavy task without `RUST_XTASK_MEMORY`.
const MEMORY: u64 = 4 << 30;

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
    if task == "fuzz" { format!("seconds={}", fuzz::SECONDS) } else { String::new() }
}

/// Reads the arguments of `task` and makes the checks of its own before it runs: Err with why it may not.
pub fn plan(task: &'static str, args: &[String], recorded: bool) -> Result<std::result::Result<Plan, String>> {
    let plain = |cap| Ok(Ok(Plan { task, cap, kind: Kind::Plain }));
    if task != "kani" && task != "fuzz" && !args.is_empty() {
        return Ok(Err(format!("{task} takes no arguments")));
    }
    match task {
        "deny" | "vet" => plain(0),
        "kani" => kani::plan(args, recorded),
        "fuzz" => fuzz::plan(args, recorded),
        _ => plain(memory(MEMORY)?),
    }
}

/// Runs the task `plan` is of; Err with why it failed.
pub fn run(plan: &Plan, run: &Run) -> Result<()> {
    match (plan.task, &plan.kind) {
        ("deny", _) => deny::run(&run.log),
        ("vet", _) => vet::run(&run.log),
        ("geiger", _) => geiger::run(plan.cap, run),
        ("miri", _) => miri::run(plan.cap, &run.log),
        ("coverage", _) => coverage::run(plan.cap, run),
        (_, Kind::Kani { only }) => kani::run(plan.cap, only, run),
        (_, Kind::Fuzz { seconds, targets, jobs, build, run: cap }) => {
            fuzz::run(*seconds, targets, *jobs, (*build, *cap), run)
        }
        _ => Err(format!("no task {}", plan.task).into()),
    }
}

fn succeed(ok: bool, otherwise: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(otherwise.into()) }
}
