//! The results of the tasks, kept with the code, and whether they still hold.
//!
//! A run is written into a directory of its own under .running/ and, when it ends, takes the place of the task's
//! directory in audit/ (or in target/audit/ with --no-record): the latest run, and the last that succeeded under
//! `last_succeeded/` if the latest failed. Its run.txt says when it ran, at what commit, with what arguments and tools and
//! how it went; its inputs.txt has the git blob of every file the task reads, which says later whether it still holds.
//! This file, like pins.rs, is an input of no task.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::run::{Log, Result, dir, parse_time, root, seconds, time};
use crate::setup::{TASKS, versions};

/// The modules of xtask that change neither how a task runs nor what it finds: what records the results and says
/// whether they hold, the pinned versions (which run.txt keeps for each task), the locks, the installing of the tools
/// and the looking for their releases.
const NOT_INPUTS: [&str; 5] =
    ["xtask/src/record.rs", "xtask/src/pins.rs", "xtask/src/lock.rs", "xtask/src/setup.rs", "xtask/src/outdated.rs"];
/// The modules of xtask only some tasks use, and those tasks.
const USED_BY: [(&str, &[&str]); 1] = [("xtask/src/json.rs", &["kani"])];

/// Whether `path`, a file of xtask, is an input of `task`: each task's module of tasks/ of that task only, a module
/// some tasks use of those, and the rest (a file added too) of every task.
fn xtask_input(task: &str, path: &str) -> bool {
    if NOT_INPUTS.contains(&path) {
        return false;
    }
    if let Some((_, tasks)) = USED_BY.iter().find(|(file, _)| *file == path) {
        return tasks.contains(&task);
    }
    match path.strip_prefix("xtask/src/tasks/").and_then(|file| file.strip_suffix(".rs")) {
        Some(owner) if TASKS.contains(&owner) => owner == task,
        _ => true,
    }
}

/// Whether `task` reads `path` (relative to the root, with "/"), so that its results stop holding when it changes.
pub fn affects(task: &str, path: &str) -> bool {
    let under = |dir: &str| path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'));
    // How every task runs: the dependencies, the toolchain, the configuration of cargo, and xtask.
    let every = path == "Cargo.lock"
        || (path.rsplit('/').next() == Some("Cargo.toml") && !under("fuzz"))
        || under(".cargo")
        || path == "rust-toolchain.toml"
        || (under("xtask") && xtask_input(task, path));
    if every {
        return true;
    }
    // The code, which is every file but those below; one added is in, so that it is run once too often rather than
    // not at all.
    let code = !doc(path) && !under("xtask") && path != "deny.toml" && !under("supply-chain") && !under("tests");
    match task {
        "deny" => ["deny.toml", "fuzz/Cargo.toml", "fuzz/Cargo.lock"].contains(&path),
        "vet" => under("supply-chain"),
        "fuzz" => code,
        "geiger" | "kani" | "miri" => code && !under("fuzz"),
        "coverage" => (code || under("tests")) && !under("fuzz"),
        _ => true,
    }
}

/// Whether `path` is of the documentation, which no task reads. No file of it is read by the code: nothing of it is
/// in an `include_str!`, `include_bytes!` or `include!`, or read by build.rs.
fn doc(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    Path::new(name).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        || path.split('/').any(|part| part.starts_with("README") || part.starts_with("LICENSE"))
        || path.starts_with(".github/")
        || [".gitignore", "clippy.toml", "rustfmt.toml"].contains(&name)
}

/// The files of the working tree, each with its git blob: uncommitted or untracked too (ignored ones aside), as
/// `git add --all` into an index of its own would have them, the real one left as it is.
pub fn tree() -> Result<BTreeMap<String, String>> {
    let index = dir("target/audit")?.join(format!("index-{}", std::process::id()));
    let real = git(&["rev-parse", "--git-path", "index"])?;
    if let Err(error) = fs::copy(root().join(real.trim()), &index)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(error.into());
    }
    let staged = git_with(&index, &["add", "--all", "--", ".", ":!audit"])
        .and_then(|_| git_with(&index, &["ls-files", "-s", "-z", "--", ".", ":!audit"]));
    let _ = fs::remove_file(&index);
    let mut files = BTreeMap::new();
    // "<mode> <blob> <stage>\t<path>\0"
    for entry in staged?.split('\0').filter(|entry| !entry.is_empty()) {
        let (fields, path) = entry.split_once('\t').ok_or("git ls-files: no tab")?;
        let blob = fields.split(' ').nth(1).ok_or("git ls-files: no blob")?;
        files.insert(path.to_owned(), blob.to_owned());
    }
    Ok(files)
}

/// The files of `tree` that `task` reads.
fn inputs<'a>(task: &str, tree: &'a BTreeMap<String, String>) -> BTreeMap<&'a str, &'a str> {
    tree.iter().filter(|(path, _)| affects(task, path)).map(|(path, blob)| (path.as_str(), blob.as_str())).collect()
}

fn git(args: &[&str]) -> Result<String> {
    run_git(Command::new("git").args(args))
}

fn git_with(index: &Path, args: &[&str]) -> Result<String> {
    run_git(Command::new("git").env("GIT_INDEX_FILE", index).args(args))
}

fn run_git(command: &mut Command) -> Result<String> {
    let output = command.current_dir(root()).stderr(Stdio::inherit()).output()?;
    if !output.status.success() {
        return Err(format!("{command:?} failed: {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Why a run may not be recorded: what is changed and not committed, which no commit's results would be of.
pub fn uncommitted() -> Result<Option<String>> {
    // Not trimmed: the first entry may begin with a space (" M file").
    let status = git(&["status", "--porcelain", "-z", "--untracked-files=all", "--", ".", ":!audit"])?;
    let mut changed = Vec::new();
    let mut entries = status.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        // "XY path"; a rename or a copy has the path it was at in an entry of its own after it.
        if entry.starts_with(['R', 'C']) {
            entries.next();
        }
        changed.extend(entry.get(3..));
    }
    Ok((!changed.is_empty()).then(|| format!("uncommitted changes: {}", list(&changed))))
}

/// Whether git tracks Cargo.lock, by which the results know whether the dependencies changed.
pub fn lock_tracked() -> Result<bool> {
    Ok(!git(&["ls-files", "--", "Cargo.lock"])?.is_empty())
}

/// "a, b and 3 more".
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [first, second, third] => format!("{first}, {second} and {third}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more", rest.len()),
    }
}

/// Where the runs of a task go: audit/, or target/audit/ when they are not recorded.
fn base(recorded: bool) -> &'static str {
    if recorded { "audit" } else { "target/audit" }
}

/// A run of a task, under way.
pub struct Run {
    pub task: &'static str,
    /// The directory it is written into.
    pub dir: PathBuf,
    /// run.log, into which its output goes, and onto the terminal.
    pub log: Log,
    recorded: bool,
    started: i64,
    clock: Instant,
    commit: String,
    arguments: String,
}

impl Run {
    /// Starts a run of `task` with `arguments` (those of the task itself, as "seconds=3600"), the leftovers of one that
    /// was stopped taken away first: to be called once the task's lock is held.
    pub fn start(task: &'static str, arguments: String, recorded: bool) -> Result<Run> {
        let dir = root().join(base(recorded)).join(".running").join(task);
        let into = root().join(base(recorded)).join(task);
        let last = dir.join("last_succeeded");
        // A run stopped while it was put in place, after the results it keeps had been moved into it.
        if !into.exists() && last.exists() {
            fs::rename(&last, &into)?;
        }
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        // When it starts, not when it ends: it does not see what changes while it runs, an hour maybe.
        let mut listed = String::new();
        for (path, blob) in inputs(task, &tree()?) {
            let _ = writeln!(listed, "{blob} {path}");
        }
        fs::write(dir.join("inputs.txt"), listed)?;
        let log = Log::create(&dir.join("run.log"), true)?;
        let commit = git(&["rev-parse", "HEAD"])?.trim().to_owned();
        Ok(Run { task, dir, log, recorded, started: seconds(), clock: Instant::now(), commit, arguments })
    }

    /// Ends the run with how it went, and puts it in the place of the task's results; the line it says.
    pub fn finish(self, outcome: &Result<()>) -> Result<String> {
        let result = match outcome {
            Ok(()) => "succeeded".to_owned(),
            Err(error) => format!("failed: {}", error.to_string().replace('\n', " ")),
        };
        let Run { task, dir, log, recorded, started, clock, commit, arguments } = self;
        drop(log);
        // No space after a field without a value.
        let arguments = if arguments.is_empty() { String::new() } else { format!(" {arguments}") };
        let text = format!(
            "started: {}\ntook: {}s\ncommit: {commit}\narguments:{arguments}\ntools: {}\nresult: {result}\n",
            time(started),
            clock.elapsed().as_secs(),
            versions(task),
        );
        fs::write(dir.join("run.txt"), text)?;
        scrub_dir(&dir)?;
        let into = root().join(base(recorded)).join(task);
        let succeeded = outcome.is_ok();
        if recorded && !succeeded && into.exists() {
            match read(&into) {
                // The last that succeeded is kept.
                Some(previous) if previous.succeeded() => fs::rename(&into, dir.join("last_succeeded"))?,
                _ => {
                    let last = into.join("last_succeeded");
                    if last.exists() {
                        fs::rename(&last, dir.join("last_succeeded"))?;
                    }
                }
            }
        }
        if into.exists() {
            fs::remove_dir_all(&into)?;
        }
        fs::rename(&dir, &into)?;
        Ok(result)
    }
}

/// Takes the paths of this machine out of the files of `dir`, which are committed: the repository's becomes ".",
/// then the home directory "~".
fn scrub_dir(dir: &Path) -> Result<()> {
    let home = env::var("HOME").unwrap_or_default();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            scrub_dir(&path)?;
            continue;
        }
        // Its paths are relative, and the root or the home could be in one ("/root" in "src/root/a.rs").
        if path.file_name().is_some_and(|name| name == "inputs.txt") {
            continue;
        }
        let text = fs::read(&path)?;
        let mut scrubbed = scrub(&text, root().to_string_lossy().as_bytes(), b".");
        if home.len() > 1 {
            scrubbed = scrub(&scrubbed, home.as_bytes(), b"~");
        }
        if scrubbed != text {
            fs::write(&path, scrubbed)?;
        }
    }
    Ok(())
}

/// `text` with `path` made `to` where it is the whole of a path, not the beginning of a longer name nor the end of one
/// ("/home/me/rext" in "/home/me/rext/src" but not in "/home/me/rext-old" or "/mnt/home/me/rext").
fn scrub(text: &[u8], path: &[u8], to: &[u8]) -> Vec<u8> {
    let in_name = |b: &u8| b.is_ascii_alphanumeric() || b"-_.".contains(b);
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let whole = text[at..].starts_with(path)
            && !text.get(at + path.len()).is_some_and(in_name)
            && !at.checked_sub(1).and_then(|before| text.get(before)).is_some_and(|b| in_name(b) || b"/~".contains(b));
        if whole && !path.is_empty() {
            out.extend_from_slice(to);
            at += path.len();
        } else {
            out.push(text[at]);
            at += 1;
        }
    }
    out
}

/// What a run.txt says.
struct Record {
    fields: BTreeMap<String, String>,
}

impl Record {
    fn get(&self, field: &str) -> &str {
        self.fields.get(field).map_or("", String::as_str)
    }

    fn succeeded(&self) -> bool {
        self.get("result") == "succeeded"
    }

    /// "2026-10-29 14:05 UTC at 1a2b3c4".
    fn when(&self) -> String {
        let started = self.get("started");
        let minutes = started.get(..16).map_or(started.to_owned(), |minutes| format!("{minutes} UTC"));
        format!("{minutes} at {}", self.get("commit").get(..7).unwrap_or("?"))
    }
}

fn read(dir: &Path) -> Option<Record> {
    let text = fs::read_to_string(dir.join("run.txt")).ok()?;
    let fields = text
        .lines()
        .filter_map(|line| line.split_once(':').map(|(field, value)| (field.to_owned(), value.trim().to_owned())))
        .collect();
    Some(Record { fields })
}

/// Whether the recorded results of `task` still hold for the working tree (as `tree` gives it) and the tools pinned
/// now: Ok with why they hold, or Err with why they do not.
pub fn holds(task: &str, arguments: &str, tree: &BTreeMap<String, String>) -> std::result::Result<String, String> {
    let dir = root().join("audit").join(task);
    let Some(record) = read(&dir) else { return Err("never run".to_owned()) };
    if !record.succeeded() {
        return Err("its latest run failed".to_owned());
    }
    if record.get("tools") != versions(task) {
        return Err("the pinned tools changed".to_owned());
    }
    if record.get("arguments") != arguments {
        return Err("recorded with different arguments".to_owned());
    }
    let Ok(listed) = fs::read_to_string(dir.join("inputs.txt")) else {
        return Err("its record has no inputs.txt".to_owned());
    };
    let then: BTreeMap<&str, &str> =
        listed.lines().filter_map(|line| line.split_once(' ').map(|(blob, path)| (path, blob))).collect();
    let now = inputs(task, tree);
    let mut changed: Vec<&str> =
        now.iter().filter(|(path, blob)| then.get(*path) != Some(blob)).map(|(path, _)| *path).collect();
    changed.extend(then.keys().filter(|path| !now.contains_key(*path)));
    if !changed.is_empty() {
        return Err(format!("changed since: {}", list(&changed)));
    }
    // The advisories change every day, the code or not.
    if task == "deny" && parse_time(record.get("started")).is_none_or(|started| seconds() - started >= 86_400) {
        return Err("over a day old".to_owned());
    }
    Ok(format!("its run of {} still holds", record.when()))
}

/// `cargo xtask status`: a line a task.
pub fn status(arguments: impl Fn(&str) -> String) -> Result<()> {
    let tree = tree()?;
    let mut out = io::stdout().lock();
    for task in TASKS {
        let dir = root().join("audit").join(task);
        let Some(record) = read(&dir) else {
            // Not println!, which panics when the pipe is closed (cargo xtask status | head).
            let _ = writeln!(out, "{task}: never run");
            continue;
        };
        let mut line = format!("{task}: {}, {}", record.get("result"), record.when());
        if let Some(last) = read(&dir.join("last_succeeded")) {
            let _ = write!(line, "; last succeeded {}", last.when());
        }
        match holds(task, &arguments(task), &tree) {
            Ok(_) => line += "; holds",
            Err(why) => line += &(String::from("; to run again: ") + &why),
        }
        let _ = writeln!(out, "{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_takes_whole_paths_only() {
        let text = b"/home/me/rext/src /home/me/rext-old /home/me/rext. /home/me/rext\n/home/me/other";
        let once = scrub(text, b"/home/me/rext", b".");
        assert_eq!(once, b"./src /home/me/rext-old /home/me/rext. .\n/home/me/other");
        assert_eq!(scrub(&once, b"/home/me", b"~"), b"./src ~/rext-old ~/rext. .\n~/other");
        assert_eq!(
            scrub(b"/mnt/home/me/x src/home/me ~/home/me", b"/home/me", b"~"),
            b"/mnt/home/me/x src/home/me ~/home/me"
        );
    }

    #[test]
    fn what_each_task_reads() {
        for task in TASKS {
            for path in
                ["Cargo.lock", "Cargo.toml", "crates/layeredconf/Cargo.toml", ".cargo/config.toml", "xtask/src/run.rs"]
            {
                assert!(affects(task, path), "{task} {path}");
            }
            for path in ["xtask/src/record.rs", "xtask/src/pins.rs", "README.md", "README/README_CN.md", "LICENSE"] {
                assert!(!affects(task, path), "{task} {path}");
            }
        }
        assert!(affects("deny", "fuzz/Cargo.lock") && affects("deny", "deny.toml"));
        assert!(!affects("deny", "crates/layeredconf/src/lib.rs"));
        assert!(affects("vet", "supply-chain/config.toml") && !affects("vet", "deny.toml"));
        assert!(affects("fuzz", "fuzz/fuzz_targets/envfile.rs") && affects("fuzz", "crates/layeredconf/src/text.rs"));
        assert!(!affects("fuzz", "tests/python/test_config.py") && !affects("fuzz", "supply-chain/audits.toml"));
        assert!(affects("miri", "crates/layeredconf/tests/config.rs") && affects("miri", "tools/unicode_table.py"));
        assert!(!affects("miri", "fuzz/fuzz_targets/envfile.rs") && !affects("kani", "fuzz/Cargo.lock"));
        assert!(affects("coverage", "tests/python/test_config.py") && !affects("kani", "tests/python/test_config.py"));
        assert!(!affects("geiger", "SECURITY.md") && !affects("fuzz", ".gitignore"));
        for path in ["xtask/src/lock.rs", "xtask/src/setup.rs", "xtask/src/outdated.rs"] {
            assert!(TASKS.iter().all(|task| !affects(task, path)), "{path}");
        }
        for path in ["xtask/Cargo.toml", "xtask/src/main.rs", "xtask/src/tasks/mod.rs", "xtask/src/tasks/new.rs"] {
            assert!(TASKS.iter().all(|task| affects(task, path)), "{path}");
        }
        assert!(affects("kani", "xtask/src/tasks/kani.rs") && !affects("fuzz", "xtask/src/tasks/kani.rs"));
        assert!(affects("miri", "xtask/src/tasks/miri.rs") && !affects("coverage", "xtask/src/tasks/miri.rs"));
        assert!(affects("kani", "xtask/src/json.rs") && !affects("deny", "xtask/src/json.rs"));
    }

    #[test]
    fn lists_are_cut_short() {
        assert_eq!(list(&["a"]), "a");
        assert_eq!(list(&["a", "b", "c"]), "a, b and c");
        assert_eq!(list(&["a", "b", "c", "d", "e"]), "a, b and 3 more");
    }
}
