//! Keeping Cua Driver current: once a day (and when the person asks, from
//! setup, the repair guide, or `agent-desk assistant driver update`) Agent-Desk
//! reads Cua's releases (`trycua/cua` on GitHub, tags `cua-driver-rs-v<x.y.z>`;
//! nightlies and drafts don't count) and, when one is newer than the
//! installed version:
//!
//! 1. downloads this Mac's archive and checks its SHA-256 against the
//!    release's published `SHA256SUMS` (and `checksums.txt` and GitHub's
//!    own digest, when given: every one must agree);
//! 2. checks the app bundle and the executable are signed by the same
//!    Developer ID team as the installed, trusted driver, and the bundle
//!    notarized (`cua::stage`);
//! 3. installs it beside the current one and runs a self-test: the driver
//!    starts, answers MCP `initialize`, and lists a harmless app's windows
//!    (Finder's; read only);
//! 4. waits until no task is acting, then switches to it with one rename
//!    (`cua::Home::switch_to`), tests it again where tasks will run it, and
//!    goes back to the previous version when that fails.
//!
//! The previous version stays for a rollback; older ones go. Cua Driver's
//! own update checks and telemetry stay off (`cua::env`). How the last
//! check went is kept in `update.json` and shown in setup and the repair
//! guide ([`Status::line`]).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use color_eyre::eyre::{Result, bail, eyre};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cua::{self, Checks, Home};

/// Cua's releases, newest first (all of Cua's products share the feed).
pub const FEED: &str = "https://api.github.com/repos/trycua/cua/releases?per_page=60";

/// How often the releases are read.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long an update waits for the tasks to stop acting before it gives
/// up until the next try.
const IDLE_WAIT: Duration = Duration::from_secs(30 * 60);

/// The checksum lists a release publishes, in the order they are trusted.
const SUMS: [&str; 2] = ["SHA256SUMS", "checksums.txt"];

/// One Cua Driver release for this Mac.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    pub version: String,
    /// This Mac's archive.
    pub archive: String,
    /// GitHub's SHA-256 of the archive, when the feed gives one.
    pub digest: Option<String>,
    /// The checksum lists it has ([`SUMS`]).
    pub sums: Vec<String>,
}

/// `version` as numbers, to compare (`0.49.10` > `0.49.9`).
pub fn parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Whether `candidate` is newer than `installed`.
pub fn newer(candidate: &str, installed: &str) -> bool {
    parts(candidate) > parts(installed)
}

/// The newest Cua Driver release in Cua's release feed (GitHub's JSON) that
/// has an archive for this Mac and a checksum list.
pub fn latest(feed: &str) -> Result<Option<Release>> {
    let releases: Vec<Value> = serde_json::from_str(feed)
        .map_err(|error| eyre!("Cua's release list isn't valid: {error}"))?;
    Ok(releases
        .iter()
        .filter_map(release)
        .max_by(|a, b| parts(&a.version).cmp(&parts(&b.version))))
}

fn release(item: &Value) -> Option<Release> {
    if item["draft"] == true {
        return None;
    }
    let version = item["tag_name"].as_str()?.strip_prefix("cua-driver-rs-v")?;
    if !cua::valid_version(version) {
        return None;
    }
    let archive = cua::archive_name(version)?;
    let assets = item["assets"].as_array()?;
    let named = |name: &str| assets.iter().find(|asset| asset["name"] == name);
    let digest = named(&archive)?["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .map(str::to_ascii_lowercase);
    let sums: Vec<String> = SUMS
        .iter()
        .filter(|name| named(name).is_some())
        .map(|name| (*name).to_owned())
        .collect();
    (!sums.is_empty()).then(|| Release {
        version: version.to_owned(),
        archive,
        digest,
        sums,
    })
}

/// The archive's SHA-256 every published source agrees on.
pub fn checksum(release: &Release, checks: &dyn Checks) -> Result<String> {
    let mut sources: Vec<(String, String)> = Vec::new();
    for list in &release.sums {
        let text = checks.text(&cua::url(&release.version, list))?;
        if let Some(hash) = cua::checksum_in(&text, &release.archive) {
            sources.push((list.clone(), hash));
        }
    }
    if sources.is_empty() {
        bail!(
            "Cua Driver {}'s checksum list doesn't name {}",
            release.version,
            release.archive
        );
    }
    if let Some(digest) = &release.digest {
        sources.push(("GitHub's digest".into(), digest.clone()));
    }
    let first = sources[0].1.clone();
    if let Some((source, _)) = sources.iter().find(|(_, hash)| *hash != first) {
        bail!(
            "Cua Driver {}'s checksums disagree ({source}); nothing was installed",
            release.version
        );
    }
    Ok(first)
}

/// How the last check went.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    NotChecked,
    UpToDate,
    Updated,
    /// A newer one waits for the tasks to stop acting.
    Waiting,
    Failed,
}

/// The last check, as `update.json` keeps it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Status {
    /// When it was read (ms since the epoch); due again [`EVERY`] later.
    #[serde(default)]
    pub checked_ms: Option<u64>,
    pub outcome: Outcome,
    /// The version tasks run after it.
    #[serde(default)]
    pub installed: Option<String>,
    /// The newest release it found.
    #[serde(default)]
    pub latest: Option<String>,
    /// The version it updated from.
    #[serde(default)]
    pub from: Option<String>,
    /// Why it failed (and the version kept).
    #[serde(default)]
    pub error: Option<String>,
}

impl Status {
    /// One line for setup and the repair guide: "Cua Driver 0.49.3 · up to
    /// date", "… · updated from 0.49.1", "… · update failed (kept 0.49.1)".
    pub fn line(&self) -> String {
        let version = self.installed.as_deref().unwrap_or("not installed");
        let state = match self.outcome {
            Outcome::NotChecked => "not checked for updates yet".to_owned(),
            Outcome::UpToDate => "up to date".to_owned(),
            Outcome::Updated => match &self.from {
                Some(from) => format!("updated from {from}"),
                None => "updated".to_owned(),
            },
            Outcome::Waiting => format!(
                "{} is ready once no task is acting",
                self.latest.as_deref().unwrap_or("an update")
            ),
            Outcome::Failed => match &self.installed {
                Some(kept) => format!("update failed (kept {kept})"),
                None => "update failed".to_owned(),
            },
        };
        format!("{} {version} · {state}", super::NAME)
    }

    /// Whether a check is due at `now_ms`.
    pub fn due(&self, now_ms: u64) -> bool {
        self.outcome == Outcome::Waiting
            || self
                .checked_ms
                .is_none_or(|at| now_ms.saturating_sub(at) >= EVERY.as_millis() as u64)
    }
}

fn status_path(home: &Home) -> std::path::PathBuf {
    home.path().join("update.json")
}

/// The last check in `home`; what is installed when there was none.
pub fn status_in(home: &Home) -> Status {
    let mut status: Status = std::fs::read_to_string(status_path(home))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    status.installed = home.current();
    status
}

fn save(home: &Home, status: &Status) {
    let saved = std::fs::create_dir_all(home.path()).and_then(|()| {
        let partial = home.path().join(".update.json.partial");
        std::fs::write(
            &partial,
            serde_json::to_vec_pretty(status).unwrap_or_default(),
        )?;
        std::fs::rename(&partial, status_path(home))
    });
    if let Err(error) = saved {
        tracing::warn!(%error, "couldn't keep Cua Driver's update status");
    }
}

/// The last check on this Mac.
pub fn status() -> Status {
    if cfg!(test) {
        return Status::default();
    }
    cua::standard()
        .map(|home| status_in(&home))
        .unwrap_or_default()
}

/// What an update needs besides the release feed: how a driver is tested,
/// and whether the tasks are idle.
pub struct Update<'a> {
    pub home: &'a Home,
    pub checks: &'a dyn Checks,
    /// Starts a driver and checks it answers (`guard::driver_self_test`).
    pub self_test: &'a dyn Fn(&Path) -> std::result::Result<(), String>,
    /// Whether no task is acting.
    pub idle: &'a dyn Fn() -> bool,
    /// How long to wait for [`Update::idle`].
    pub idle_wait: Duration,
    pub now_ms: u64,
}

/// Reads the feed and updates when a newer release is there; the status,
/// kept.
pub fn run(update: &Update, progress: &mut dyn FnMut(&str)) -> Status {
    let home = update.home;
    let before = home.current();
    let mut status = Status {
        checked_ms: Some(update.now_ms),
        installed: before.clone(),
        ..Status::default()
    };
    match step(update, &mut status, progress) {
        Ok(outcome) => status.outcome = outcome,
        Err(error) => {
            tracing::warn!(%error, kept = ?home.current(), "Cua Driver didn't update");
            status.outcome = Outcome::Failed;
            status.error = Some(format!("{error:#}"));
        }
    }
    status.installed = home.current();
    if status.outcome == Outcome::Waiting {
        // Not checked yet: the next try comes in the hour.
        status.checked_ms = None;
    }
    save(home, &status);
    status
}

fn step(update: &Update, status: &mut Status, progress: &mut dyn FnMut(&str)) -> Result<Outcome> {
    let home = update.home;
    progress("Looking for a newer Cua Driver…");
    let feed = update.checks.text(FEED)?;
    let Some(release) = latest(&feed)? else {
        bail!("Cua's release list has no Cua Driver for this Mac");
    };
    status.latest = Some(release.version.clone());
    let current = home.current();
    if current
        .as_deref()
        .is_some_and(|current| !newer(&release.version, current))
    {
        return Ok(Outcome::UpToDate);
    }
    let team = cua::trusted_team(home, update.checks);
    let sha256 = checksum(&release, update.checks)?;
    let earlier = status_in(home);
    let staged = home.executable(&release.version).filter(|_| {
        // Staged and tested by an earlier try that waited for the tasks.
        earlier.outcome == Outcome::Waiting
            && earlier.latest.as_deref() == Some(release.version.as_str())
    });
    let path = match staged {
        Some(path) if update.checks.team(&path).ok().as_deref() == Some(team.as_str()) => path,
        _ => cua::stage(
            home,
            update.checks,
            &release.version,
            &release.archive,
            &sha256,
            &team,
            progress,
        )?,
    };
    progress(&format!("Testing Cua Driver {}…", release.version));
    if let Err(why) = (update.self_test)(&path) {
        let _ = std::fs::remove_dir_all(home.path().join(&release.version));
        bail!("Cua Driver {} failed its self-test: {why}", release.version);
    }
    if !wait_idle(update.idle, update.idle_wait) {
        return Ok(Outcome::Waiting);
    }
    home.switch_to(&release.version)?;
    let switched = home
        .installed()
        .ok_or_else(|| eyre!("Cua Driver {} isn't where tasks run it", release.version));
    if let Err(why) = switched.and_then(|path| (update.self_test)(&path).map_err(|why| eyre!(why)))
    {
        let back = home.roll_back()?;
        bail!(
            "Cua Driver {} failed after the switch ({why:#}); back on {back}",
            release.version
        );
    }
    status.from = current;
    home.prune();
    tracing::info!(version = release.version, from = ?status.from, "updated Cua Driver");
    Ok(Outcome::Updated)
}

/// Waits up to `limit` for `idle`; whether it came.
fn wait_idle(idle: &dyn Fn() -> bool, limit: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        if idle() {
            return true;
        }
        if started.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_secs(15).min(limit));
    }
}

/// Goes back to the previous version (`agent-desk assistant driver
/// rollback`); the version now running.
pub fn roll_back() -> Result<String> {
    let home = cua::standard().ok_or_else(|| eyre!("no application support folder"))?;
    let back = home.roll_back()?;
    let mut status = status_in(&home);
    status.error = None;
    status.from = None;
    save(&home, &status);
    Ok(back)
}

/// Whether no task acts on this Mac, as any process can tell: none holds a
/// window (the desk's apps aside) or moves the real pointer.
pub fn idle_across() -> bool {
    crate::assistant::guard::hands_off().is_none()
        && crate::assistant::guard::leases()
            .iter()
            .all(|lease| lease.task == crate::assistant::guard::DESK)
}

/// [`idle_across`], and no task runs or waits in this Agent-Desk.
pub fn idle_in_app() -> bool {
    !crate::assistant::service::busy() && idle_across()
}

/// One check now, with the real feed, checks, and self-test (setup's and
/// the repair guide's Update button, the CLI). Waits for `idle` as long as
/// `idle_wait`. Installs the pinned release first when none is.
pub fn check_now(
    idle: &dyn Fn() -> bool,
    idle_wait: Duration,
    progress: &mut dyn FnMut(&str),
) -> Result<Status> {
    if !cfg!(target_os = "macos") {
        bail!("Cua Driver runs on macOS only");
    }
    let home = cua::standard().ok_or_else(|| eyre!("no application support folder"))?;
    let _lock = Lock::take(&home)?;
    if home.installed().is_none() {
        cua::install_pinned(&home, &cua::System, progress)?;
    }
    Ok(run(
        &Update {
            home: &home,
            checks: &cua::System,
            self_test: &crate::assistant::guard::driver_self_test,
            idle,
            idle_wait,
            now_ms: crate::desktop::events::now_ms(),
        },
        progress,
    ))
}

/// One update at a time, across Agent-Desk and its CLI: `<home>/.updating`
/// holds the pid that runs one.
struct Lock(std::path::PathBuf);

impl Lock {
    fn take(home: &Home) -> Result<Self> {
        std::fs::create_dir_all(home.path())?;
        let path = home.path().join(".updating");
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    let _ = write!(file, "{}", std::process::id());
                    return Ok(Self(path));
                }
                Err(_) => {
                    let holder = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|text| text.trim().parse::<u32>().ok());
                    if holder.is_some_and(crate::assistant::guard::process_alive) {
                        bail!("Cua Driver is being updated already");
                    }
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        bail!("couldn't take Cua Driver's update lock")
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// In Agent-Desk (`guard::startup`): installs Cua Driver when it isn't, then
/// checks for a newer one daily, in the background.
pub fn start_daily() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if cfg!(test) || !cfg!(target_os = "macos") || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("agent-desk-cua-update".into())
        .spawn(|| {
            loop {
                let due = status().due(crate::desktop::events::now_ms());
                if due || cua::installed().is_none() {
                    match check_now(&idle_in_app, IDLE_WAIT, &mut |step| {
                        tracing::info!(step, "Cua Driver")
                    }) {
                        Ok(status) => tracing::info!(line = status.line(), "Cua Driver"),
                        Err(error) => {
                            tracing::warn!(%error, "Cua Driver's update check didn't run")
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(60 * 60));
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't start Cua Driver's update checks");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
