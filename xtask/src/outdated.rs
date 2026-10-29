//! When a pinned version is due an upgrade: one with a known vulnerability at once, one a month older than a release of
//! its tool, a nightly a month old.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::json::{self, Value};
use crate::run::{self, Result, date, day, today};
use crate::tools::{self, NIGHTLY, TOOLS, Tool};

/// Who asks, as crates.io wants to know: the project, without an address of mail.
const USER_AGENT: &str = "layeredconf-xtask (https://github.com/lutianyu2001/layeredconf)";
/// How much older than a release, or how old for a nightly, a pinned version may be.
const MONTH: i64 = 30;

struct Due {
    /// The crate, or "Rust" for the nightly.
    what: String,
    from: String,
    /// What to upgrade to, if there is something.
    to: Option<String>,
    why: String,
}

impl Due {
    fn line(&self) -> String {
        match &self.to {
            Some(to) => format!("{} {} → {to}: {}", self.what, self.from, self.why),
            None => format!("{} {}: {} (no release to upgrade to yet)", self.what, self.from, self.why),
        }
    }
}

/// `cargo xtask outdated [--vulns]`: says what is due and, in a terminal, upgrades what the user agrees to. Ok(false)
/// when something is due that was not upgraded since nobody could be asked.
pub fn outdated(vulns_only: bool) -> Result<bool> {
    let mut due = Vec::new();
    for tool in TOOLS {
        due.extend(check(tool, !vulns_only)?);
    }
    stamp("vulns");
    if !vulns_only {
        due.extend(nightly_due());
        stamp("releases");
    }
    if due.is_empty() {
        println!("outdated: no pinned version is due an upgrade");
        return Ok(true);
    }
    for due in &due {
        println!("{}", due.line());
    }
    if !io::stdin().is_terminal() {
        return Ok(false);
    }
    let path = run::root().join("xtask/src/tools.rs");
    let mut source = fs::read_to_string(&path)?;
    let mut upgraded = false;
    for due in &due {
        let Some(to) = &due.to else { continue };
        if !ask(&format!("upgrade {} from {} to {to}?", due.what, due.from))? {
            continue;
        }
        let (old, new) = if due.what == "Rust" {
            // A nightly may lack a component (Miri breaks now and then): then the one of a day before.
            let Some(toolchain) = (1..=7)
                .map(|back| format!("nightly-{}", date(today() - back)))
                .find(|toolchain| tools::install_nightly(toolchain))
            else {
                eprintln!("outdated: no nightly of the past week installs with the components needed; {NIGHTLY} stays");
                continue;
            };
            (format!("NIGHTLY: &str = \"{}\"", due.from), format!("NIGHTLY: &str = \"{toolchain}\""))
        } else {
            (
                format!("krate: \"{}\", version: \"{}\"", due.what, due.from),
                format!("krate: \"{}\", version: \"{to}\"", due.what),
            )
        };
        if !source.contains(&old) {
            return Err(format!("{} has no {old} to rewrite", path.display()).into());
        }
        source = source.replace(&old, &new);
        upgraded = true;
    }
    if upgraded {
        fs::write(&path, source)?;
        println!("outdated: {} rewritten; installing the versions now pinned", path.display());
        // A new xtask, built with the new constants.
        let status = run::cargo(None, &["xtask", "setup"]).status()?;
        if !status.success() {
            return Err(format!("cargo xtask setup failed: {status}").into());
        }
    }
    Ok(true)
}

/// For the tasks other than outdated: when the last check is older than `RUST_XTASK_VULNS_EVERY` days (1 by default) for
/// vulnerabilities, or `RUST_XTASK_RELEASES_EVERY` days (7) for releases, checks again and says in a line what is due.
/// Silent when it cannot reach crates.io or OSV.
pub fn notice() {
    let vulns = stale("vulns", "RUST_XTASK_VULNS_EVERY", 1);
    let releases = stale("releases", "RUST_XTASK_RELEASES_EVERY", 7);
    if !vulns && !releases {
        return;
    }
    let mut due = Vec::new();
    for tool in TOOLS {
        match check(tool, releases) {
            Ok(found) => due.extend(found),
            Err(_) => return,
        }
    }
    stamp("vulns");
    if releases {
        due.extend(nightly_due());
        stamp("releases");
    }
    for due in due {
        println!("xtask: {}; cargo xtask outdated to upgrade", due.line());
    }
}

/// Whether `tool` is due: a vulnerability always, a release of a month later than the pinned one if `releases`.
fn check(tool: Tool, releases: bool) -> Result<Option<Due>> {
    let vulns = vulnerabilities(tool.krate, tool.version)?;
    if vulns.is_empty() && !releases {
        return Ok(None);
    }
    let all = released(tool.krate)?;
    let pinned = all.iter().find(|release| release.version == tool.version);
    let latest = all
        .iter()
        .filter(|release| !release.yanked)
        .filter_map(|release| Some((stable(&release.version)?, release)))
        .max_by(|a, b| a.0.cmp(&b.0));
    let newer = latest.filter(|(numbers, _)| Some(numbers) > stable(tool.version).as_ref()).map(|(_, release)| release);
    let due = |why: String| Due {
        what: tool.krate.to_owned(),
        from: tool.version.to_owned(),
        to: newer.map(|release| release.version.clone()),
        why,
    };
    if !vulns.is_empty() {
        return Ok(Some(due(format!("known vulnerabilities {}", vulns.join(", ")))));
    }
    Ok(match (pinned, newer) {
        (Some(pinned), _) if pinned.yanked => Some(due("yanked".to_owned())),
        (Some(pinned), Some(newer)) if newer.day - pinned.day >= MONTH => Some(due(format!(
            "{} came out on {}, {} days after {}",
            newer.version,
            date(newer.day),
            newer.day - pinned.day,
            tool.version
        ))),
        _ => None,
    })
}

fn nightly_due() -> Option<Due> {
    let age = today() - day(NIGHTLY.strip_prefix("nightly-")?)?;
    (age >= MONTH).then(|| Due {
        what: "Rust".to_owned(),
        from: NIGHTLY.to_owned(),
        to: Some(format!("nightly-{}", date(today() - 1))),
        why: format!("{age} days old"),
    })
}

/// The numbers of a version to compare by; None for a pre-release.
fn stable(version: &str) -> Option<Vec<u64>> {
    // Build metadata may have a hyphen ("1.1.8+spec-1.1.0"); a pre-release has one before it.
    let numbers = version.split('+').next()?;
    if numbers.contains('-') {
        return None;
    }
    numbers.split('.').map(|number| number.parse().ok()).collect()
}

struct Release {
    version: String,
    day: i64,
    yanked: bool,
}

/// Every release of `krate`, as crates.io lists them.
fn released(krate: &str) -> Result<Vec<Release>> {
    let first = format!("https://crates.io/api/v1/crates/{krate}/versions");
    let mut url = first.clone();
    let mut releases = Vec::new();
    loop {
        // crates.io asks for no more than a request a second.
        thread::sleep(Duration::from_secs(1));
        let page = fetch(&url, None)?;
        for version in page.get("versions").map_or(&[][..], Value::as_array) {
            let number = version.get("num").and_then(Value::as_str);
            let created = version.get("created_at").and_then(Value::as_str).and_then(day);
            if let (Some(number), Some(created)) = (number, created) {
                let yanked = version.get("yanked").is_some_and(Value::is_true);
                releases.push(Release { version: number.to_owned(), day: created, yanked });
            }
        }
        match page.get("meta").and_then(|meta| meta.get("next_page")).and_then(Value::as_str) {
            Some(next) => url = format!("{first}{next}"),
            None => return Ok(releases),
        }
    }
}

/// The vulnerabilities OSV, which has the advisories of `RustSec`, knows in `krate` at `version`.
fn vulnerabilities(krate: &str, version: &str) -> Result<Vec<String>> {
    let query = format!(r#"{{"package": {{"name": "{krate}", "ecosystem": "crates.io"}}, "version": "{version}"}}"#);
    let answer = fetch("https://api.osv.dev/v1/query", Some(&query))?;
    let vulns = answer.get("vulns").map_or(&[][..], Value::as_array);
    Ok(vulns.iter().filter_map(|vuln| vuln.get("id").and_then(Value::as_str)).map(str::to_owned).collect())
}

/// The JSON at `url`, posting `body` if there is one, with curl.
fn fetch(url: &str, body: Option<&str>) -> Result<Value> {
    let mut command = Command::new("curl");
    command.args(["--silent", "--show-error", "--fail", "--location", "--max-time", "20", "--user-agent", USER_AGENT]);
    if body.is_some() {
        command.args(["--header", "Content-Type: application/json", "--data-binary", "@-"]);
    }
    let mut child = command.arg(url).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    if let (Some(mut stdin), Some(body)) = (child.stdin.take(), body) {
        stdin.write_all(body.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(format!("{url}: {}", String::from_utf8_lossy(&output.stderr).trim()).into());
    }
    json::parse(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| format!("{url}: the answer is not JSON").into())
}

fn ask(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn stamp_path(kind: &str) -> Option<PathBuf> {
    run::audit_dir().ok().map(|dir| dir.join(format!("checked-{kind}")))
}

/// Whether the last check of `kind` is older than the days `variable` says, or than `days`.
fn stale(kind: &str, variable: &str, days: u64) -> bool {
    let days = env::var(variable).ok().and_then(|value| value.parse().ok()).unwrap_or(days);
    let Some(path) = stamp_path(kind) else { return false };
    let age =
        fs::metadata(path).and_then(|metadata| metadata.modified()).ok().and_then(|modified| modified.elapsed().ok());
    age.is_none_or(|age| age >= Duration::from_secs(days * 86_400))
}

fn stamp(kind: &str) {
    if let Some(path) = stamp_path(kind) {
        let _ = fs::write(path, run::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_leaves_out_pre_releases() {
        assert_eq!(stable("0.20.2"), Some(vec![0, 20, 2]));
        assert_eq!(stable("1.1.8+spec-1.1.0"), Some(vec![1, 1, 8]));
        assert_eq!(stable("0.21.0-rc.1"), None);
        assert!(stable("0.10.0") > stable("0.9.12"));
    }
}
