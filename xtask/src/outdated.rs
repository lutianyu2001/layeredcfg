//! When a pinned version is due an upgrade: one with a known vulnerability at once; one when the latest release of its
//! tool came out a month or more after it, and there is a release to upgrade to; a nightly a month old.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::json::{self, Value};
use crate::pins::{NIGHTLY, TOOLS, Tool};
use crate::run::{self, Result, date, day, today};
use crate::setup;

/// Who asks, as crates.io wants to know: the project, without an address of mail.
const USER_AGENT: &str = "layeredconf-xtask (https://github.com/lutianyu2001/layeredconf)";
/// How much later than the pinned version the latest release must have come out, or how old a nightly must be, for
/// it to be due.
const MONTH: i64 = 30;
/// How old a release must be to be upgraded to (one to be withdrawn, or a crate taken over, is mostly found out by
/// then), and how old the nightly upgraded to is (a nightly's regressions are mostly mended by then).
const WEEK: i64 = 7;

/// A pinned version due an upgrade.
struct Due {
    /// The crate, or "Rust" for the nightly.
    what: String,
    from: String,
    /// What to upgrade to, if there is something; there is for all but a version with a vulnerability.
    to: Option<String>,
    why: String,
}

impl Due {
    fn line(&self) -> String {
        match &self.to {
            Some(to) => format!("{} {} → {to}: {}", self.what, self.from, self.why),
            None => format!("{} {}: {}, and no release to upgrade to yet", self.what, self.from, self.why),
        }
    }
}

/// What the checks found: the versions due, and lines about those that are not due though there is a newer release.
#[derive(Default)]
struct Found {
    due: Vec<Due>,
    notes: Vec<String>,
}

/// `cargo xtask outdated [--vulns]`: says what is due and, in a terminal, upgrades what the user agrees to, writing it
/// into pins.rs. The exit code, and whether something was upgraded (for setup to install). Exit 2 for what is due and
/// was not asked about, nobody being there, or has a vulnerability and nothing to upgrade to.
pub fn outdated(vulns_only: bool) -> Result<(u8, bool)> {
    let mut found = Found::default();
    for tool in TOOLS {
        check(tool, !vulns_only, &mut found)?;
    }
    stamp("vulns");
    if !vulns_only {
        found.due.extend(nightly_due());
        stamp("releases");
    }
    for note in &found.notes {
        println!("outdated: {note}");
    }
    if found.due.is_empty() {
        println!("outdated: no pinned version is due an upgrade");
        return Ok((0, false));
    }
    for due in &found.due {
        println!("outdated: {}", due.line());
    }
    // Nothing to upgrade to: CI is to stay red until there is.
    let stuck = found.due.iter().any(|due| due.to.is_none());
    if !io::stdin().is_terminal() {
        return Ok((2, false));
    }
    let path = run::root().join("xtask/src/pins.rs");
    let mut upgraded = false;
    for due in &found.due {
        let Some(to) = &due.to else { continue };
        let source = fs::read_to_string(&path)?;
        let pinned = if due.what == "Rust" {
            format!("NIGHTLY: &str = \"{}\"", due.from)
        } else {
            format!("krate: \"{}\", version: \"{}\"", due.what, due.from)
        };
        if !source.contains(&pinned) {
            eprintln!("outdated: {} has no {pinned} to rewrite; {} is left as it is", path.display(), due.what);
            continue;
        }
        if !ask(&format!("upgrade {} from {} to {to}?", due.what, due.from))? {
            continue;
        }
        let new = if due.what == "Rust" {
            // Installed before it is written: a nightly may lack a component (Miri breaks now and then), and then the
            // one of a day before is tried, up to a week before.
            let Some(toolchain) = (WEEK..=2 * WEEK)
                .map(|back| format!("nightly-{}", date(today() - back)))
                .find(|toolchain| setup::install_nightly(toolchain))
            else {
                eprintln!(
                    "outdated: no nightly from {to} to a week before installs with the components needed; {NIGHTLY} stays"
                );
                continue;
            };
            format!("NIGHTLY: &str = \"{toolchain}\"")
        } else {
            format!("krate: \"{}\", version: \"{to}\"", due.what)
        };
        // Written at once, so that an upgrade agreed to stays when a later one goes wrong.
        fs::write(&path, source.replace(&pinned, &new))?;
        println!("outdated: {} now pins {}", path.display(), new);
        upgraded = true;
    }
    Ok((if stuck { 2 } else { 0 }, upgraded))
}

/// For the tasks other than outdated: when the last check is older than `RUST_XTASK_VULNS_EVERY` days (1 by default)
/// for vulnerabilities, or `RUST_XTASK_RELEASES_EVERY` days (7) for releases, checks again and says in a line what is
/// due. Silent when it cannot reach crates.io or OSV.
pub fn notice() {
    let vulns = stale("vulns", "RUST_XTASK_VULNS_EVERY", 1);
    let releases = stale("releases", "RUST_XTASK_RELEASES_EVERY", 7);
    if !vulns && !releases {
        return;
    }
    // Stamped whether or not the checks go through, so that a machine offline does not wait for them every time.
    stamp("vulns");
    if releases {
        stamp("releases");
    }
    let mut found = Found::default();
    for tool in TOOLS {
        if check(tool, releases, &mut found).is_err() {
            return;
        }
    }
    if releases {
        found.due.extend(nightly_due());
    }
    for due in found.due {
        println!("xtask: {}; cargo xtask outdated to upgrade", due.line());
    }
}

/// Whether `tool` is due: with a vulnerability always; if `releases`, when its latest release came out a month after
/// it and there is a release to upgrade to (a note otherwise).
fn check(tool: Tool, releases: bool, found: &mut Found) -> Result<()> {
    let vulns = vulnerabilities(tool.krate, tool.version)?;
    if vulns.is_empty() && !releases {
        return Ok(());
    }
    let all = released(tool.krate)?;
    let to = upgrade(tool, &all)?.map(|release| release.version.clone());
    let due =
        |why: String, to: Option<String>| Due { what: tool.krate.to_owned(), from: tool.version.to_owned(), to, why };
    if !vulns.is_empty() {
        found.due.push(due(format!("known vulnerabilities {}", vulns.join(", ")), to));
        return Ok(());
    }
    let Some(pinned) = all.iter().find(|release| release.version == tool.version) else { return Ok(()) };
    if pinned.yanked {
        match to {
            Some(to) => found.due.push(due("yanked".to_owned(), Some(to))),
            None => {
                found.notes.push(format!("{} {}: yanked, and no release to upgrade to yet", tool.krate, tool.version));
            }
        }
        return Ok(());
    }
    // Of the releases: no pre-release, nothing yanked.
    let latest = all
        .iter()
        .filter(|release| !release.yanked)
        .filter_map(|release| Some((stable(&release.version)?, release)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, release)| release);
    let Some(latest) = latest.filter(|latest| latest.day - pinned.day >= MONTH) else { return Ok(()) };
    let why = format!(
        "{} came out on {}, {} days after {}",
        latest.version,
        date(latest.day),
        latest.day - pinned.day,
        tool.version
    );
    match to {
        Some(to) => found.due.push(due(why, Some(to))),
        None => found.notes.push(format!(
            "{} {}: {why}, and there is no release to upgrade to yet: none newer is a week old, not yanked, x.y.z \
             and without a known vulnerability",
            tool.krate, tool.version
        )),
    }
    Ok(())
}

/// The release to upgrade `tool` to: the latest one later than the pinned one that is at least a week old, not
/// yanked, a plain x.y.z, and with no vulnerability known.
fn upgrade(tool: Tool, all: &[Release]) -> Result<Option<&Release>> {
    let Some(pinned) = stable(tool.version) else { return Ok(None) };
    let mut candidates: Vec<(Vec<u64>, &Release)> = all
        .iter()
        .filter(|release| !release.yanked && today() - release.day >= WEEK && plain(&release.version))
        .filter_map(|release| Some((stable(&release.version)?, release)))
        .filter(|(numbers, _)| *numbers > pinned)
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, release) in candidates.into_iter().take(5) {
        if vulnerabilities(tool.krate, &release.version)?.is_empty() {
            return Ok(Some(release));
        }
    }
    Ok(None)
}

/// Whether a version is digits and two dots only, as it may be written into pins.rs.
fn plain(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// The nightly, if it is a month old: due an upgrade to the one of a week before today. Nothing is installed to tell.
fn nightly_due() -> Option<Due> {
    let age = today() - day(NIGHTLY.strip_prefix("nightly-")?)?;
    (age >= MONTH).then(|| Due {
        what: "Rust".to_owned(),
        from: NIGHTLY.to_owned(),
        to: Some(format!("nightly-{}", date(today() - WEEK))),
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

/// The JSON at `url`, posting `body` if there is one, with curl (no crate of the dependencies does HTTPS).
fn fetch(url: &str, body: Option<&str>) -> Result<Value> {
    let mut command = Command::new("curl");
    command.args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https", "--proto-redir", "=https"]);
    command.args(["--max-time", "20", "--user-agent", USER_AGENT]);
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

/// target/audit/checked-<kind>: losing it costs a check more.
fn stamp_path(kind: &str) -> Option<PathBuf> {
    run::dir("target/audit").ok().map(|dir| dir.join(format!("checked-{kind}")))
}

/// Whether the last check of `kind` is older than the days `variable` says, or than `days`.
fn stale(kind: &str, variable: &str, days: u64) -> bool {
    let days = env::var(variable).ok().and_then(|value| value.parse().ok()).unwrap_or(days);
    let Some(path) = stamp_path(kind) else { return false };
    let age =
        fs::metadata(path).and_then(|metadata| metadata.modified()).ok().and_then(|modified| modified.elapsed().ok());
    age.is_none_or(|age| age >= Duration::from_secs(days.saturating_mul(86_400)))
}

fn stamp(kind: &str) {
    if let Some(path) = stamp_path(kind) {
        let _ = fs::write(path, run::time(run::seconds()));
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
        assert!(plain("0.20.2") && !plain("0.20") && !plain("0.20.2+x") && !plain("1.2.3\""));
    }
}
