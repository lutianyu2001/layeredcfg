//! Running the tools: from where, under what cap of memory, with their output kept; and the time.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The root of the workspace, which is that of the repository too; xtask works from it.
pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new("/"))
}

/// The directory `path` names under the root, made if it is not there.
pub fn dir(path: &str) -> Result<PathBuf> {
    let dir = root().join(path);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// cargo, with the toolchain given (the one rust-toolchain.toml pins if None).
pub fn cargo(toolchain: Option<&str>, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    // The rustup proxy runs xtask with the toolchain rust-toolchain.toml pins in these, which come before that file
    // and so would pin every cargo run from here, of another directory too, and every tool cargo install builds.
    // +toolchain comes before them.
    command.env_remove("RUSTUP_TOOLCHAIN").env_remove("RUSTUP_TOOLCHAIN_SOURCE");
    if let Some(toolchain) = toolchain {
        command.arg(format!("+{toolchain}"));
    }
    command.args(args);
    command
}

/// Whether the heavy tasks run without a cap, as `RUST_XTASK_NO_SCOPE=1` asks (in CI, where a runner is a machine of
/// its own).
pub fn no_scope() -> bool {
    env::var("RUST_XTASK_NO_SCOPE").is_ok_and(|value| value == "1")
}

/// This program, to run again inside a scope: as /proc/<pid>/exe, which is the program this process runs while it
/// lives, though cargo builds xtask anew meanwhile (any `cargo xtask` after a change of its source does), which
/// deletes the file it was started from and makes `current_exe()` a path to nothing.
pub fn this() -> PathBuf {
    PathBuf::from(format!("/proc/{}/exe", std::process::id()))
}

/// What the cap of a scope is set to, to see from inside that it caps.
const PROBE: u64 = 1 << 30;

/// Whether there is a systemd user scope that caps memory.
pub fn has_scope() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        // A scope may start and still not cap anything (cgroup v1, or no memory controller given to the user's
        // manager): the cap must be seen from inside, which this program reads there.
        let mut probe = Command::new(this());
        probe.arg("__scope-probe");
        scoped(&probe, PROBE, false)
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|output| output.status.success() && output.stdout.trim_ascii() == PROBE.to_string().as_bytes())
    })
}

/// `cargo xtask __scope-probe`, run in a scope: prints the cap of memory of the cgroup it is in.
pub fn scope_probe() -> Result<()> {
    println!("{}", fs::read_to_string(cgroup()?.join("memory.max"))?.trim());
    Ok(())
}

/// The directory of the cgroup (v2) this process is in.
pub fn cgroup() -> Result<PathBuf> {
    let line = fs::read_to_string("/proc/self/cgroup")?;
    // "0::/user.slice/…/run-r1234.scope"
    let path = line.lines().find_map(|line| line.strip_prefix("0::")).ok_or("no cgroup v2 in /proc/self/cgroup")?;
    Ok(Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

/// `command` in a systemd user scope whose memory is capped at `bytes`, without swap, so that going past it kills the
/// command alone; as it is when `RUST_XTASK_NO_SCOPE=1`. With `oom_continue`, an OOM kills the largest process alone
/// (systemd's default stops the whole scope), so that what is left may say how far the memory went.
pub fn capped(command: Command, bytes: u64, oom_continue: bool) -> Command {
    if no_scope() { command } else { scoped(&command, bytes, oom_continue) }
}

fn scoped(command: &Command, bytes: u64, oom_continue: bool) -> Command {
    let mut scope = Command::new("systemd-run");
    scope.args(["--user", "--scope", "--quiet", "-p"]).arg(format!("MemoryMax={bytes}"));
    scope.args(["-p", "MemorySwapMax=0"]);
    if oom_continue {
        scope.args(["-p", "OOMPolicy=continue"]);
    }
    scope.arg("--");
    scope.arg(command.get_program()).args(command.get_args());
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => scope.env(name, value),
            None => scope.env_remove(name),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        scope.current_dir(dir);
    }
    scope
}

/// Bytes from a size written as systemd writes it: "4G", "1536M", "800K", or bytes; powers of 1024.
pub fn bytes(size: &str) -> Option<u64> {
    let size = size.trim();
    let (number, shift) = match size.as_bytes().last()? {
        b'K' => (&size[..size.len() - 1], 10),
        b'M' => (&size[..size.len() - 1], 20),
        b'G' => (&size[..size.len() - 1], 30),
        b'T' => (&size[..size.len() - 1], 40),
        _ => (size, 0),
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse::<u64>().ok()?.checked_mul(1 << shift)
}

/// `bytes` to read: "4.0 GiB", "84 MiB".
pub fn size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)] // a figure to read
    let mib = bytes as f64 / 1_048_576.0;
    if mib >= 1024.0 { format!("{:.1} GiB", mib / 1024.0) } else { format!("{mib:.0} MiB") }
}

/// The cap a task has from `RUST_XTASK_MEMORY`, else `default`.
pub fn memory(default: u64) -> Result<u64> {
    match env::var("RUST_XTASK_MEMORY") {
        Ok(value) => Ok(bytes(&value).ok_or_else(|| format!("RUST_XTASK_MEMORY={value} is no size (4G, 1536M)"))?),
        Err(_) => Ok(default),
    }
}

/// A log file, which every command run with it writes its output into.
pub struct Log {
    file: Arc<Mutex<File>>,
    echo: bool,
}

impl Log {
    /// The log at `path`, emptied; what is run with it goes onto the terminal as well if `echo`.
    pub fn create(path: &Path, echo: bool) -> Result<Log> {
        Ok(Log { file: Arc::new(Mutex::new(File::create(path)?)), echo })
    }

    /// Writes a line into the log, and onto the terminal.
    pub fn say(&self, line: &str) {
        println!("{line}");
        let _ = writeln!(lock(&self.file), "{line}");
    }

    /// Runs `command` until it ends, its output going into the log.
    pub fn run(&self, command: Command) -> Result<ExitStatus> {
        Ok(self.capture(command, false)?.0)
    }

    /// Runs `command` until it ends, its output going into the log, and its standard output into `out` as well.
    pub fn run_into(&self, command: Command, out: &Path) -> Result<ExitStatus> {
        let (status, stdout) = self.capture(command, true)?;
        fs::write(out, stdout)?;
        Ok(status)
    }

    fn capture(&self, mut command: Command, keep: bool) -> Result<(ExitStatus, Vec<u8>)> {
        writeln!(lock(&self.file), "$ {}", shown(&command))?;
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let echo = self.echo;
        let out = child.stdout.take().map(|from| self.copy(from, echo.then(|| Box::new(io::stdout()) as _), keep));
        let err = child.stderr.take().map(|from| self.copy(from, echo.then(|| Box::new(io::stderr()) as _), false));
        let status = child.wait()?;
        let mut kept = Vec::new();
        for copying in [out, err].into_iter().flatten() {
            kept.extend(copying.join().unwrap_or_default());
        }
        writeln!(lock(&self.file), "[{status}]")?;
        Ok((status, kept))
    }

    fn copy(
        &self,
        mut from: impl Read + Send + 'static,
        mut echo: Option<Box<dyn Write + Send>>,
        keep: bool,
    ) -> thread::JoinHandle<Vec<u8>> {
        let file = Arc::clone(&self.file);
        thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buffer = [0; 8192];
            while let Ok(n @ 1..) = from.read(&mut buffer) {
                let _ = lock(&file).write_all(&buffer[..n]);
                if let Some(echo) = echo.as_mut() {
                    let _ = echo.write_all(&buffer[..n]).and_then(|()| echo.flush());
                }
                if keep {
                    kept.extend_from_slice(&buffer[..n]);
                }
            }
            kept
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

/// Seconds since 1970-01-01 00:00 UTC.
pub fn seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

/// Days since 1970-01-01, in UTC.
pub fn today() -> i64 {
    seconds() / 86_400
}

/// A time as "2026-10-29 14:05:09 UTC".
pub fn time(seconds: i64) -> String {
    let (day, second) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    format!("{} {:02}:{:02}:{:02} UTC", date(day), second / 3600, second % 3600 / 60, second % 60)
}

/// The seconds since 1970 of a time `time` wrote.
pub fn parse_time(text: &str) -> Option<i64> {
    let clock = text.get(11..19)?;
    let mut fields = clock.split(':').map(|field| field.parse::<i64>().ok());
    let (hour, minute, second) = (fields.next()??, fields.next()??, fields.next()??);
    Some(day(text)? * 86_400 + hour * 3600 + minute * 60 + second)
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
        let now = seconds();
        assert_eq!(parse_time(&time(now)), Some(now));
        assert_eq!(time(86_399), "1970-01-01 23:59:59 UTC");
    }

    #[test]
    fn sizes_as_systemd_writes_them() {
        assert_eq!(bytes("4G"), Some(4 << 30));
        assert_eq!(bytes("1536M"), Some(1536 << 20));
        assert_eq!(bytes("1024"), Some(1024));
        for wrong in ["", "G", "4g", "-1G", "1.5G", "99999999999999999999T"] {
            assert_eq!(bytes(wrong), None, "{wrong}");
        }
        assert_eq!(size(84 << 20), "84 MiB");
        assert_eq!(size(16 << 30), "16.0 GiB");
    }
}
