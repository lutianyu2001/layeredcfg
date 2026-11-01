//! Which tasks may run at once.
//!
//! By default one at a time: each holds audit/lock alone. With `RUST_XTASK_PARALLEL=1` they hold it shared, and run
//! side by side while the caps of memory of those running fit in the machine, Kani and the fuzzing never together.
//! Each task holds a lock of its own as well, audit/.running/<task>.lock, which has its cap in it; audit/budget is held
//! while one is checked against those.

use std::env;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, Write};

use crate::run::{Result, dir, size};

/// What is left to the system and the desktop, past the caps of the tasks.
const KEPT: u64 = 4 << 30;

/// Whether the tasks may run side by side, as `RUST_XTASK_PARALLEL=1` asks.
pub fn parallel() -> bool {
    env::var("RUST_XTASK_PARALLEL").is_ok_and(|value| value == "1")
}

fn open(path: &str) -> Result<File> {
    let (parent, name) = path.rsplit_once('/').unwrap_or((".", path));
    // Not truncated: what another holder wrote is read through it.
    Ok(OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir(parent)?.join(name))?)
}

/// audit/lock: alone, or shared if `shared`; an error if that cannot be had now.
pub fn audit(shared: bool) -> Result<File> {
    let file = open("audit/lock")?;
    let locked = if shared { file.try_lock_shared() } else { file.try_lock() };
    match locked {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(format!(
            "the audit is in use: another task, setup or outdated holds audit/lock{}",
            if shared { "" } else { " (RUST_XTASK_PARALLEL=1 lets tasks run side by side)" }
        )
        .into()),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}

/// The lock of `task` while it runs, with its cap of memory written into it; Err with why it may not run now.
pub fn task(task: &str, cap: u64) -> Result<std::result::Result<File, String>> {
    let mut file = open(&format!("audit/.running/{task}.lock"))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            // A task checking its budget holds this one shared for a moment, and checks only with audit/budget held:
            // once that is had, nobody checks.
            let budget = open("audit/budget")?;
            budget.lock()?;
            let again = file.try_lock();
            drop(budget);
            match again {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Ok(Err(format!("{task} is running already"))),
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    file.set_len(0)?;
    file.write_all(cap.to_string().as_bytes())?;
    file.flush()?;
    if parallel() {
        let budget = open("audit/budget")?;
        budget.lock()?;
        let fits = fits(task, cap)?;
        drop(budget);
        if let Err(why) = fits {
            return Ok(Err(why));
        }
    }
    Ok(Ok(file))
}

/// Whether `task`, capped at `cap`, fits beside the tasks running.
fn fits(task: &str, cap: u64) -> Result<std::result::Result<(), String>> {
    let total = meminfo("MemTotal:")?;
    let mut running = Vec::new();
    for entry in fs::read_dir(dir("audit/.running")?)? {
        let path = entry?.path();
        let Some(other) = path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_suffix(".lock"))
        else {
            continue;
        };
        if other == task {
            continue;
        }
        let mut file = File::open(&path)?;
        // One that nobody holds was left by a run that was stopped.
        match file.try_lock_shared() {
            Ok(()) => continue,
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        let mut text = String::new();
        file.rewind()?;
        file.read_to_string(&mut text)?;
        // Empty or unreadable: it has not written its cap yet, so it is taken to want everything.
        running.push((other.to_owned(), text.trim().parse::<u64>().unwrap_or(total)));
    }
    let heavy = |name: &str| name == "kani" || name == "fuzz";
    if heavy(task)
        && let Some((other, _)) = running.iter().find(|(other, _)| heavy(other))
    {
        return Ok(Err(format!("{other} is running, and Kani and the fuzzing never run at once")));
    }
    let taken: u64 = running.iter().map(|(_, cap)| cap).sum();
    let room = total.saturating_sub(KEPT);
    if taken.saturating_add(cap) > room {
        let names: Vec<&str> = running.iter().map(|(name, _)| name.as_str()).collect();
        return Ok(Err(format!(
            "{} running ({}) and {} for {task} are past the {} the tasks may have (the memory less {}), by {}",
            size(taken),
            names.join(", "),
            size(cap),
            size(room),
            size(KEPT),
            size(taken + cap - room)
        )));
    }
    let available = meminfo("MemAvailable:")?;
    if cap > available {
        return Ok(Err(format!("{task} is capped at {}, and {} of memory is available", size(cap), size(available))));
    }
    Ok(Ok(()))
}

/// A field of /proc/meminfo, in bytes.
fn meminfo(field: &str) -> Result<u64> {
    let text = fs::read_to_string("/proc/meminfo")?;
    let kilobytes = text
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .and_then(|rest| rest.trim().strip_suffix("kB"))
        .and_then(|number| number.trim().parse::<u64>().ok())
        .ok_or_else(|| format!("no {field} in /proc/meminfo"))?;
    Ok(kilobytes * 1024)
}
