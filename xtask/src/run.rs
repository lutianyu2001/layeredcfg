//! Running the tools: from where, under what cap of memory, with their output kept, and the date.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The root of the repository.
pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask is a directory of the repository")
}

/// target/audit/, where the logs go.
pub fn audit_dir() -> Result<PathBuf> {
    let dir = root().join("target/audit");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// cargo, run from the root, with the toolchain given (the one rust-toolchain.toml pins if None).
pub fn cargo(toolchain: Option<&str>, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    // The rustup proxy, not the cargo that runs xtask (CARGO), so that +toolchain works; and without the toolchain
    // rustup may have set for xtask.
    command.env_remove("RUSTUP_TOOLCHAIN").current_dir(root());
    if let Some(toolchain) = toolchain {
        command.arg(format!("+{toolchain}"));
    }
    command.args(args);
    command
}

/// `command` in a systemd user scope whose memory is capped at `memory` (or `RUST_XTASK_MEMORY`), without swap, so that
/// going past it kills the command alone; as it is where there is no such scope.
pub fn capped(command: Command, memory: &str) -> Command {
    if !has_scope() {
        return command;
    }
    let memory = env::var("RUST_XTASK_MEMORY").unwrap_or_else(|_| memory.to_owned());
    let mut scope = Command::new("systemd-run");
    scope.args(["--user", "--scope", "--quiet", "-p"]).arg(format!("MemoryMax={memory}"));
    scope.args(["-p", "MemorySwapMax=0", "--"]);
    around(scope, &command)
}

fn has_scope() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        if env::var_os("RUST_XTASK_NO_SCOPE").is_some() {
            return false;
        }
        let works = Command::new("systemd-run")
            .args(["--user", "--scope", "--quiet", "-p", "MemoryMax=64M", "true"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !works {
            eprintln!("xtask: no systemd user scope here, so the heavy tasks run without a cap of memory");
        }
        works
    })
}

/// `/usr/bin/time -v command`, to have the peak of memory in the log, if there is such a program.
pub fn timed(command: Command) -> Command {
    let time = Path::new("/usr/bin/time");
    if !time.exists() {
        return command;
    }
    let mut timed = Command::new(time);
    timed.arg("-v");
    around(timed, &command)
}

/// `outer` running `command`, in the environment and the directory of `command`.
fn around(mut outer: Command, command: &Command) -> Command {
    outer.arg(command.get_program()).args(command.get_args());
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => outer.env(name, value),
            None => outer.env_remove(name),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        outer.current_dir(dir);
    }
    outer
}

/// The peak of memory `/usr/bin/time -v` wrote into `log`, as "2.1 GB".
pub fn peak_memory(log: &str) -> Option<String> {
    let kilobytes: u64 =
        log.lines().find_map(|line| line.trim().strip_prefix("Maximum resident set size (kbytes): "))?.parse().ok()?;
    #[allow(clippy::cast_precision_loss)] // as a figure to read
    Some(format!("{:.1} GB", kilobytes as f64 / 1_048_576.0))
}

/// A log file, which every command run with it writes its output into.
pub struct Log {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

impl Log {
    /// The log at `path`, emptied.
    pub fn create(path: PathBuf) -> Result<Log> {
        let file = Arc::new(Mutex::new(File::create(&path)?));
        Ok(Log { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> String {
        fs::read_to_string(&self.path).unwrap_or_default()
    }

    /// Runs `command` until it ends, its output going into the log and, if `echo`, onto the terminal as well.
    pub fn run(&self, mut command: Command, echo: bool) -> Result<ExitStatus> {
        writeln!(lock(&self.file), "$ {}", shown(&command))?;
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let out = child.stdout.take().map(|from| self.copy(from, echo.then(|| Box::new(io::stdout()) as Box<_>)));
        let err = child.stderr.take().map(|from| self.copy(from, echo.then(|| Box::new(io::stderr()) as Box<_>)));
        let status = child.wait()?;
        for copying in [out, err].into_iter().flatten() {
            let _ = copying.join();
        }
        writeln!(lock(&self.file), "[{status}]")?;
        Ok(status)
    }

    fn copy(
        &self,
        mut from: impl Read + Send + 'static,
        mut echo: Option<Box<dyn Write + Send>>,
    ) -> thread::JoinHandle<()> {
        let file = Arc::clone(&self.file);
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(n @ 1..) = from.read(&mut buffer) {
                let _ = lock(&file).write_all(&buffer[..n]);
                if let Some(echo) = echo.as_mut() {
                    let _ = echo.write_all(&buffer[..n]).and_then(|()| echo.flush());
                }
            }
        })
    }
}

fn lock(file: &Mutex<File>) -> std::sync::MutexGuard<'_, File> {
    file.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn shown(command: &Command) -> String {
    let mut words = vec![command.get_program().to_string_lossy().into_owned()];
    words.extend(command.get_args().map(|arg| arg.to_string_lossy().into_owned()));
    words.join(" ")
}

/// Says how a task went, and adds it to target/audit/summary.txt.
pub fn record(task: &str, outcome: &str) {
    println!("{task}: {outcome}");
    let line = format!("{} {task}: {outcome}\n", now());
    let written = audit_dir().and_then(|dir| {
        let mut summary = OpenOptions::new().create(true).append(true).open(dir.join("summary.txt"))?;
        Ok(summary.write_all(line.as_bytes())?)
    });
    if let Err(error) = written {
        eprintln!("xtask: cannot add to the summary: {error}");
    }
}

/// Days since 1970-01-01, in UTC.
pub fn today() -> i64 {
    seconds() / 86_400
}

fn seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// The time now, as "2026-10-29 14:05 UTC".
pub fn now() -> String {
    let seconds = seconds();
    format!("{} {:02}:{:02} UTC", date(seconds / 86_400), seconds % 86_400 / 3600, seconds % 3600 / 60)
}

/// "YYYY-MM-DD" of a day since 1970-01-01.
pub fn date(days: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The day since 1970-01-01 a text beginning with "YYYY-MM-DD" names.
pub fn day(text: &str) -> Option<i64> {
    let mut fields = text.get(..10)?.split('-').map(|field| field.parse::<i64>().ok());
    let (year, month, day) = (fields.next()??, fields.next()??, fields.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_dates() {
        assert_eq!(day("1970-01-01"), Some(0));
        assert_eq!(day("2026-06-09T18:37:27.158092Z"), Some(20_613));
        assert_eq!(date(20_613), "2026-06-09");
        for days in 0..100_000 {
            assert_eq!(day(&date(days)), Some(days));
        }
        assert_eq!(day("2026-13-01"), None);
        assert_eq!(day("nightly"), None);
    }

    #[test]
    fn peak_memory_reads_time() {
        assert_eq!(peak_memory("\tMaximum resident set size (kbytes): 2179252\n").as_deref(), Some("2.1 GB"));
        assert_eq!(peak_memory("nothing"), None);
    }
}
