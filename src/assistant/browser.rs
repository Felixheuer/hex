//! Hidden browsers for web work: agent-browser (Vercel Labs), one session
//! per task, so twenty web tasks run at once without a window on the
//! person's screen and without waiting on the desktop's turn. Tasks get it
//! as a second tool beside the guard's desktop tools and are told to prefer
//! it for anything on the web.
//!
//! agent-browser runs headless Chrome for Testing (`agent-browser install`)
//! or, for clean reading, Lightpanda (a light headless browser, about a
//! tenth of Chrome's memory) behind a daemon per session, and has its own
//! MCP server (`agent-browser mcp`). A task gets that server through `hex
//! assistant browser-mcp --task <id>` ([`proxy`]), which pins every call to
//! one of the task's own browsers ([`sessions`]: its main one and up to
//! seven named ones) in Agent-Desk's own namespace and withholds the arguments that
//! would show a window, reach another task, or persist state.
//!
//! **Sign-ins.** A hidden browser has none of the person's cookies. The
//! person signs in once: by taking the logins of one of their Chrome
//! profiles over ([`accounts::import_from_chrome`], refreshed daily), or by
//! hand in a visible window of Agent-Desk's own ([`sign_in`]). agent-browser keeps
//! them under one restore key, and every task's browser starts from a
//! read-only copy of them (it never saves back): in real Chrome when it asks
//! for `signedIn`, else in the light engine, which loads them too
//! (`assistant.browser_signed_in_by_default`; `signedIn: false` starts one
//! clean).
//!
//! OWNER: browser.

pub mod accounts;
pub mod consent;
mod proxy;
pub mod sessions;
pub mod stream;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use crate::assistant::backends::{self, McpServer, run, search};

pub use proxy::serve;
pub use sessions::{Engine, max_sessions};

/// The server's name in each agent's config: its tools are then
/// `mcp__agent-browser__agent_browser_*` (Claude Code), `agent_browser_*`
/// (Codex), and `agent-browser_agent_browser_*` (OpenCode).
pub const MCP_NAME: &str = "agent-browser";

/// Agent-Desk's own agent-browser namespace: its sessions and saved state stay
/// apart from the person's own agent-browser use.
const NAMESPACE: &str = "hex";

/// The sign-in window's session, and the restore key every task loads.
const SIGN_IN: &str = "hex-sign-in";

/// A task's browser that saw no command for this long closes itself, in
/// case the task's end never closed it.
const IDLE_MS: &str = "1800000";

/// How long one agent-browser command may take (a cold Chrome launch is a
/// few seconds).
const COMMAND_LIMIT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentBrowser {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Its own Chrome is downloaded (`agent-browser install`).
    pub browser_ready: bool,
}

/// agent-browser's path, version, and whether it has a browser to run.
/// Blocks briefly (two quick commands); off the UI thread.
pub fn agent_browser() -> AgentBrowser {
    let Some(path) = executable() else {
        return AgentBrowser::default();
    };
    let version = backends::version_of(&path).ok().flatten();
    let browser_ready = browser_installed(&path).unwrap_or(false);
    AgentBrowser {
        path: Some(path),
        version,
        browser_ready,
    }
}

/// The agent-browser executable, if installed.
pub fn executable() -> Option<PathBuf> {
    search::find("agent-browser")
}

/// `agent-browser doctor --offline --quick --json`: whether a Chrome it can
/// launch is there.
fn browser_installed(path: &Path) -> Option<bool> {
    let mut command = base_command(path);
    command.args(["doctor", "--offline", "--quick", "--json"]);
    let ran = run::bounded(command, Duration::from_secs(15)).ok()?;
    chrome_installed(&ran.stdout)
}

/// `chrome.installed` in `doctor --json`'s checks.
pub fn chrome_installed(doctor: &str) -> Option<bool> {
    let report = last_json(doctor)?;
    let check = report
        .get("checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("id").and_then(Value::as_str) == Some("chrome.installed"))?;
    Some(check.get("status").and_then(Value::as_str) == Some("pass"))
}

/// `brew install agent-browser` (else npm), then `agent-browser install`
/// (its Chrome); blocks. What isn't missing isn't installed again.
pub fn install() -> Result<()> {
    install_with(|_| {})
}

/// [`install`], handing `progress` each line of the installers' output.
pub fn install_with(progress: impl FnMut(&str)) -> Result<()> {
    backends::apply_fix_with(backends::Fix::InstallAgentBrowser, progress)
}

/// Lightpanda, if installed (`~/.local/bin/lightpanda` from its installer).
pub fn lightpanda() -> Option<PathBuf> {
    search::find("lightpanda")
}

/// The engine behind a task's browsers: Chrome, whose pages stream live
/// pictures to the overlay (the person watches every browser as a mini);
/// Lightpanda draws none, so it only takes the overflow past a task's
/// share of Chrome ([`sessions::Sessions`]).
pub fn clean_engine() -> Engine {
    Engine::Chrome
}

/// Installs Lightpanda with its official script (into `~/.local/bin`);
/// blocks. Nothing happens when it's installed.
pub fn install_lightpanda_with(progress: impl FnMut(&str)) -> Result<()> {
    backends::apply_fix_with(backends::Fix::InstallLightpanda, progress)
}

/// The browser tool for `task`: its own browsers, closed when the task
/// ends ([`close`]). `None` when agent-browser isn't installed, or while
/// `background_browser` is off.
pub fn mcp_server(task: &str) -> Option<McpServer> {
    let enabled = crate::app_settings::assistant_runtime()
        .is_none_or(|(_, settings)| settings.background_browser);
    if !enabled {
        return None;
    }
    let browser = executable()?;
    let hex = std::env::current_exe().ok()?;
    Some(server(hex, task, &browser))
}

fn server(hex: PathBuf, task: &str, browser: &Path) -> McpServer {
    McpServer {
        command: hex,
        args: ["assistant", "browser-mcp", "--task", task]
            .map(str::to_owned)
            .to_vec(),
        path_env: search::path_value(&[browser.to_owned()]),
    }
}

/// Starts `task`'s main browser now, exactly as its first browser call
/// would start it (`Sessions::default_launch`): a job's first page then
/// opens in a warm browser while its agent is still starting. Measured with
/// agent-browser 0.42 on this Mac: a cold first page took 0.31 s in
/// Lightpanda and 0.56–1.09 s in Chrome, a warm one 0.10 s and 0.12 s
/// (twenty helpers' at once 0.36 s, against about 4 s cold). Only a light
/// Lightpanda browser is started for every job, and only while memory is
/// fine; real Chrome with the person's accounts only when `request` names a
/// site they signed in to for Agent-Desk ([`names_site`] among
/// [`signed_in_sites`]): the task's
/// first `signedIn: true` browser is that one
/// (`Sessions::pin`). The task's end closes them like any other
/// ([`close`]), and they close themselves when unused for half an hour.
/// Runs on threads of its own; starting one that runs already changes
/// nothing (it reads the page's address and never navigates).
pub fn prewarm_for(task: &str, request: &str) {
    let enabled = crate::app_settings::assistant_runtime()
        .is_none_or(|(_, settings)| settings.background_browser);
    if !enabled
        || crate::assistant::capacity::pressure() != crate::assistant::capacity::Pressure::Normal
    {
        return;
    }
    let Some(path) = executable() else {
        return;
    };
    let sessions =
        sessions::Sessions::new(task, clean_engine()).with_accounts(accounts_by_default());
    let mut starts = Vec::new();
    if clean_engine() == Engine::Lightpanda {
        starts.push((session(task), sessions.default_launch()));
    }
    if signed_in() && names_site(request, &signed_in_sites()) {
        let (name, launch) = sessions.warm_chrome();
        starts.push((name, launch));
    }
    for (session, launch) in starts {
        let path = path.clone();
        // The task's first browser call waits for this start instead of
        // racing it (two starts of one session: the second fails).
        let warming = Warming::mark(&session);
        let spawned = std::thread::Builder::new()
            .name("hex-browser-prewarm".into())
            .spawn(move || {
                let _warming = warming;
                let started = std::time::Instant::now();
                let (args, env) = prewarm_command(&launch);
                let mut command = command(&path, &session);
                command.envs(env).arg("--json").args(&args);
                match run::bounded(command, COMMAND_LIMIT) {
                    Ok(ran) if !ran.timed_out() => {
                        // The sign-ins came back with the saved state's last
                        // page: a browser started ahead shows a blank page
                        // until its task opens one (the task's first call
                        // waits for this start, so it's still unused here).
                        let url = last_json(&ran.stdout)
                            .and_then(|reply| reply["data"]["url"].as_str().map(str::to_owned));
                        if blank_needed(url.as_deref(), used(&session)) {
                            let _ = call(&path, &session, &["open", "about:blank"]);
                        }
                        tracing::debug!(
                            session,
                            ms = started.elapsed().as_millis() as u64,
                            "started a job's browser ahead"
                        )
                    }
                    Ok(_) | Err(_) => {
                        tracing::debug!(session, "couldn't start a job's browser ahead")
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::debug!(%error, "couldn't start a browser ahead");
        }
    }
}

/// Words before a site's name that make it the place to go (`in Example`,
/// `bei Kiosk`, `open Example`): grammar, never a list of sites.
const TOWARD: &[&str] = &[
    "in", "im", "auf", "bei", "zu", "on", "at", "into", "to", "open", "öffne", "öffnen", "visit",
    "besuche",
];

/// Words after a site's name that make it a site (`Example account`).
const SITE_WORDS: &[&str] = &[
    "site", "website", "seite", "webseite", "account", "konto", "login",
];

/// Whether a browser started ahead goes to a blank page: it shows a page
/// (`url`, the restored state's last one) and its task never used it (a
/// used one is the task's, and keeps its page).
fn blank_needed(url: Option<&str>, used: bool) -> bool {
    !used && url.is_some_and(|url| !url.is_empty() && url != "about:blank")
}

/// Whether `request` names one of `sites` (hosts such as
/// `mail.example.com`) as a place to go: the host itself or an address of
/// the same site (`example.de`), or the site's own name, the label before
/// its suffix (`example`, a whole word of at least four letters), used as
/// a destination ([`TOWARD`] before it, or [`SITE_WORDS`] after it). A
/// site's name alone is often a word of its own ("google it", "Google mal
/// nach …" means search), so it alone starts nothing.
pub fn names_site(request: &str, sites: &[String]) -> bool {
    let text = request.to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
        .map(|word| word.trim_matches(|c: char| c == '.' || c == '-'))
        .filter(|word| !word.is_empty())
        .collect();
    sites.iter().any(|site| {
        let host = site.trim_start_matches('.').to_lowercase();
        if host.is_empty() {
            return false;
        }
        let name = site_name(&host).filter(|name| name.chars().count() >= 4);
        words.iter().enumerate().any(|(at, word)| {
            if *word == host {
                return true;
            }
            let Some(name) = name else {
                return false;
            };
            if word.contains('.') {
                // An address: the same site under another suffix or host.
                return site_name(word) == Some(name);
            }
            *word == name
                && (at
                    .checked_sub(1)
                    .is_some_and(|before| TOWARD.contains(&words[before]))
                    || words
                        .get(at + 1)
                        .is_some_and(|after| SITE_WORDS.contains(after)))
        })
    })
}

/// A host's own name: the label before its suffix (`example` in
/// `mail.example.com` and in `shop.example.co.uk`).
fn site_name(host: &str) -> Option<&str> {
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    let n = labels.len();
    if n < 2 {
        return None;
    }
    // A two-letter country suffix under a short second level (co.uk, com.au).
    let suffix = if n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3 {
        2
    } else {
        1
    };
    labels.get(n.checked_sub(suffix + 1)?).copied()
}

/// The sites the person signed in to for Agent-Desk: those in the sign-in store
/// (never a list of sites), read again only when a state file changed. A
/// request naming one ([`names_site`]) is worth a warm real Chrome with
/// their accounts.
pub fn signed_in_sites() -> Vec<String> {
    type Stamp = Vec<(std::ffi::OsString, Option<std::time::SystemTime>, u64)>;
    static CACHE: std::sync::Mutex<Option<(Stamp, Vec<String>)>> = std::sync::Mutex::new(None);
    let mut stamp: Stamp = state_dir()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_sign_in_state(&entry.file_name().to_string_lossy()))
        .map(|entry| {
            let meta = entry.metadata().ok();
            (
                entry.file_name(),
                meta.as_ref().and_then(|meta| meta.modified().ok()),
                meta.map_or(0, |meta| meta.len()),
            )
        })
        .collect();
    stamp.sort();
    if let Ok(cache) = CACHE.lock()
        && let Some((seen, sites)) = cache.as_ref()
        && *seen == stamp
    {
        return sites.clone();
    }
    let sites = saved_sites();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((stamp, sites.clone()));
    }
    sites
}

/// The agent-browser arguments and environment that start a browser with
/// `launch` (`Sessions::default_launch`'s `restore`, `restoreSave`, and
/// `extraArgs`) and only read its page's address: on a browser that runs
/// already they change nothing, so a prewarm that comes late never takes a
/// task's page away.
fn prewarm_command(
    launch: &serde_json::Map<String, Value>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut args: Vec<String> = launch
        .get("extraArgs")
        .and_then(Value::as_array)
        .map(|extra| {
            extra
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    args.extend(["get".to_owned(), "url".to_owned()]);
    let mut env = Vec::new();
    for (key, name) in [
        ("restore", "AGENT_BROWSER_RESTORE"),
        ("restoreSave", "AGENT_BROWSER_RESTORE_SAVE"),
    ] {
        if let Some(value) = launch.get(key).and_then(Value::as_str) {
            env.push((name.to_owned(), value.to_owned()));
        }
    }
    (args, env)
}

/// A browser being started ahead (`prewarm_for`): a marker file under
/// `<support>/assistant/browser-warming/<session>` while it starts, which
/// the task's browser proxy (another process) waits on before its first
/// call to that session. Removed when dropped.
pub(crate) struct Warming(Option<PathBuf>);

/// How long a first call waits for a start ahead, at most.
const WARM_WAIT: Duration = Duration::from_secs(20);

impl Warming {
    fn dir() -> Option<PathBuf> {
        crate::app_paths::support_dir()
            .ok()
            .map(|support| support.join("assistant").join("browser-warming"))
    }

    fn mark(session: &str) -> Self {
        let path = Self::dir().map(|dir| dir.join(session));
        if let Some(path) = &path {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")));
            let _ = std::fs::write(path, std::process::id().to_string());
        }
        Self(path)
    }
}

impl Drop for Warming {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Which of the task's browsers the task has used: a marker file under
/// `<support>/assistant/browser-used/<session>` written by its browser
/// proxy (another process) on the first tool call pinned to it. Only those
/// show as minis (the cards, the desk, the wall); one started ahead
/// ([`prewarm_for`]) and never used stays hidden, and closes with its task.
pub(crate) struct Used(PathBuf);

impl Used {
    /// This Mac's markers.
    pub(crate) fn here() -> Option<Self> {
        crate::app_paths::support_dir()
            .ok()
            .map(|support| Self(support.join("assistant").join("browser-used")))
    }

    /// `session` is used from now on.
    pub(crate) fn mark(&self, session: &str) {
        let path = self.0.join(session);
        if path.exists() {
            return;
        }
        let _ = std::fs::create_dir_all(&self.0);
        let _ = std::fs::write(path, std::process::id().to_string());
    }

    pub(crate) fn has(&self, session: &str) -> bool {
        self.0.join(session).exists()
    }

    /// `session` closed: the next one of that name starts unused.
    pub(crate) fn forget(&self, session: &str) {
        let _ = std::fs::remove_file(self.0.join(session));
    }

    /// The ones of `sessions` that were used, in their order.
    pub(crate) fn only_used(&self, sessions: Vec<String>) -> Vec<String> {
        sessions
            .into_iter()
            .filter(|session| self.has(session))
            .collect()
    }
}

/// Whether `session` was used (`Used`); unknown markers count as used, so
/// a missing support folder never hides a task's browser.
pub(crate) fn used(session: &str) -> bool {
    Used::here().is_none_or(|used| used.has(session))
}

/// Waits while `session` is being started ahead (at most [`WARM_WAIT`],
/// and never on a marker older than a start can take).
pub(crate) fn wait_for_warmup(session: &str) {
    let Some(path) = Warming::dir().map(|dir| dir.join(session)) else {
        return;
    };
    let started = std::time::Instant::now();
    loop {
        let fresh = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < COMMAND_LIMIT);
        if !fresh || started.elapsed() >= WARM_WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Closes every browser of `task` (nothing is saved), which ended in
/// `lane`, unless that was a quick (Here) turn: its browsers stay open on
/// the person's desk, live at the orb, until they let them go
/// ([`close_now`]). Decided by the lane the task ended in, never by the
/// conversation's first origin: a job escalated from a hold closes its
/// research browsers. Blocks for a moment; quick when it never opened one.
/// Browsers the task started ahead and never used close either way.
pub fn close(task: &str, lane: crate::assistant::types::Lane) {
    if stays_on_desk(lane, || has_open(task)) {
        if let Some(path) = executable() {
            let (_, unused) = split_used(running(&path, task), used);
            close_sessions(&path, &unused);
        }
        crate::assistant::guard::desk_keep_browser(task);
        tracing::info!(task, "a quick turn's browser stays on the desk");
        return;
    }
    close_now(task);
}

/// `sessions` as (used, unused) by `used`.
fn split_used(sessions: Vec<String>, used: impl Fn(&str) -> bool) -> (Vec<String>, Vec<String>) {
    sessions.into_iter().partition(|session| used(session))
}

/// Closes `sessions` at once, and forgets that they were used.
fn close_sessions(path: &Path, sessions: &[String]) {
    let markers = Used::here();
    std::thread::scope(|scope| {
        for session in sessions {
            let markers = &markers;
            scope.spawn(move || {
                match call(path, session, &["close"]) {
                    Ok(_) => tracing::debug!(session, "closed a task's browser"),
                    Err(error) => {
                        tracing::warn!(session, error, "could not close a task's browser")
                    }
                }
                if let Some(markers) = markers {
                    markers.forget(session);
                }
            });
        }
    });
}

/// Whether a task that ended in `lane` keeps its browsers on the desk:
/// only a quick turn's, and only when one is open (`open`, asked last).
fn stays_on_desk(lane: crate::assistant::types::Lane, open: impl FnOnce() -> bool) -> bool {
    lane == crate::assistant::types::Lane::Here && open()
}

/// Whether `task` has a browser open now that it used (one only started
/// ahead keeps nothing on the desk).
fn has_open(task: &str) -> bool {
    executable().is_some_and(|path| running(&path, task).iter().any(|session| used(session)))
}

/// Closes every browser of `task` now, desk or not.
pub fn close_now(task: &str) {
    crate::assistant::guard::desk_release_browser(task);
    let Some(path) = executable() else {
        return;
    };
    let mut open = running(&path, task);
    let main = session(task);
    if !open.contains(&main) {
        // Listed or not, the main one is closed (a quick no-op if absent).
        open.push(main);
    }
    close_sessions(&path, &open);
}

/// Reads a session's page through agent-browser, for [`consent`].
pub(crate) struct PageProbe {
    pub path: PathBuf,
}

impl consent::Probe for PageProbe {
    fn inspect(&self, session: &str, focus: Option<&str>) -> Option<consent::Inspection> {
        if let Some(selector) = focus {
            call(&self.path, session, &["focus", selector]).ok()?;
        }
        let data = call(&self.path, session, &["eval", consent::INSPECT]).ok()?;
        serde_json::from_value(data.get("result")?.clone()).ok()
    }

    fn label(&self, session: &str, selector: &str) -> Option<String> {
        let read = |args: &[&str], key: &str| {
            call(&self.path, session, args)
                .ok()?
                .get(key)?
                .as_str()
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
        };
        read(&["get", "attr", selector, "aria-label"], "value")
            .or_else(|| read(&["get", "text", selector], "text"))
            .or_else(|| read(&["get", "attr", selector, "value"], "value"))
    }
}

/// The autonomy mode (Settings, read fresh) and what `task` may do.
pub(crate) fn policy(task: &str) -> consent::Policy {
    let task = task.to_owned();
    Box::new(move || {
        let mode = crate::app_settings::assistant_runtime()
            .map(|(_, settings)| settings.autonomy)
            .or_else(|| {
                crate::app_settings::AppSettings::read()
                    .ok()
                    .map(|settings| settings.assistant.autonomy)
            })
            .unwrap_or_default();
        (mode, crate::assistant::autonomy::read(&task))
    })
}

/// `task`'s browsers that agent-browser has open.
pub(crate) fn running(path: &Path, task: &str) -> Vec<String> {
    running_of(path, &[task.to_owned()])
        .remove(task)
        .unwrap_or_default()
}

/// Each of `tasks`' browsers that agent-browser has open, from one listing
/// (one process however many tasks are asked about).
pub(crate) fn running_of(
    path: &Path,
    tasks: &[String],
) -> std::collections::HashMap<String, Vec<String>> {
    let mut found: std::collections::HashMap<String, Vec<String>> = tasks
        .iter()
        .map(|task| (task.clone(), Vec::new()))
        .collect();
    let Some(first) = tasks.first() else {
        return found;
    };
    // `session list` ignores AGENT_BROWSER_NAMESPACE (agent-browser 0.42),
    // so the namespace goes as a flag too: without it the list is empty and
    // no task shows its browsers.
    let listed = call(
        path,
        &session(first),
        &["--namespace", NAMESPACE, "session", "list"],
    )
    .unwrap_or_default();
    let open: Vec<&str> = listed
        .get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    for task in tasks {
        let mine = found.entry(task.clone()).or_default();
        mine.extend(
            open.iter()
                .filter(|session| sessions::belongs(task, session))
                .map(|session| (*session).to_owned()),
        );
    }
    found
}

/// `task`'s session name: agent-browser takes letters, digits, `-`, `_`.
pub fn session(task: &str) -> String {
    let id: String = task
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    format!("hex-{id}")
}

/// Opens Agent-Desk's own browser in a visible window at `url`, so the person can
/// sign in to the sites tasks should use. Its cookies and storage are saved
/// as they change and when [`finish_sign_in`] closes it; every task's
/// browser starts with them from then on. Returns once the window is open.
pub fn sign_in(url: Option<&str>) -> Result<()> {
    let path = executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    // Private before the window saves anything into it: the person may
    // never press Done.
    if let Some(dir) = state_dir() {
        let _ = std::fs::create_dir_all(&dir);
    }
    private_state();
    call(
        &path,
        SIGN_IN,
        &["--headed", "open", url.unwrap_or("about:blank")],
    )
    .map(|_| ())
    .map_err(|error| eyre!("Agent-Desk's browser didn't open: {error}"))
}

/// Saves and closes the sign-in window.
pub fn finish_sign_in() -> Result<()> {
    let path = executable().ok_or_else(|| eyre!("agent-browser isn't installed"))?;
    call(&path, SIGN_IN, &["close"])
        .map(|_| ())
        .map_err(|error| eyre!("Agent-Desk's browser didn't close: {error}"))?;
    private_state();
    Ok(())
}

/// Websites with saved browser data, not a claim that their login is still valid.
pub fn saved_sites() -> Vec<String> {
    let mut sites = std::collections::BTreeSet::new();
    if let Some(dir) = state_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries
            .flatten()
            .filter(|entry| is_sign_in_state(&entry.file_name().to_string_lossy()))
        {
            if let Ok(bytes) = std::fs::read(entry.path())
                && let Ok(state) = serde_json::from_slice::<Value>(&bytes)
            {
                sites.extend(sites_in_state(&state));
            }
        }
    }
    sites.into_iter().collect()
}

fn sites_in_state(state: &Value) -> Vec<String> {
    let mut sites = std::collections::BTreeSet::new();
    for cookie in state["cookies"].as_array().into_iter().flatten() {
        if cookie["value"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
            && let Some(domain) = cookie["domain"].as_str()
            && let Ok(url) = url::Url::parse(&format!("https://{}", domain.trim_start_matches('.')))
            && let Some(host) = url.host_str()
        {
            sites.insert(host.to_owned());
        }
    }
    for origin in state["origins"].as_array().into_iter().flatten() {
        if origin["localStorage"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
            && let Some(origin) = origin["origin"].as_str()
            && let Ok(url) = url::Url::parse(origin)
            && let Some(host) = url.host_str()
        {
            sites.insert(host.to_owned());
        }
    }
    sites.into_iter().collect()
}

/// A typed website; never launch file, script, or credential-bearing URLs.
pub fn sign_in_url(input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let url = url::Url::parse(&address).map_err(|_| "Enter a valid website address")?;
    if input.is_empty()
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Enter a valid website address");
    }
    Ok(url.into())
}

/// Whether browser data is saved for any website; it can expire independently.
/// Cheap: asked as every job and browser starts, and the saved state is
/// often a megabyte of cookies, so a state file of any real size answers
/// without reading it; only a tiny one is read to see whether it is blank.
pub fn signed_in() -> bool {
    let Some(dir) = state_dir() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries
        .flatten()
        .filter(|entry| is_sign_in_state(&entry.file_name().to_string_lossy()))
        .any(|entry| {
            let size = entry.metadata().map_or(0, |metadata| metadata.len());
            size > BLANK_STATE_BYTES
                || std::fs::read(entry.path())
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|state| !sites_in_state(&state).is_empty())
        })
}

/// A saved state this small may hold no site (`{"cookies":[],"origins":[]}`);
/// one larger holds at least a cookie or some storage.
const BLANK_STATE_BYTES: u64 = 512;

/// Whether a task's browser that asks for nothing starts with the person's
/// accounts: the setting is on (read fresh) and some are saved.
pub fn accounts_by_default() -> bool {
    let on = crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.browser_signed_in_by_default)
        .or_else(|| {
            crate::app_settings::AppSettings::read()
                .ok()
                .map(|settings| settings.assistant.browser_signed_in_by_default)
        })
        .unwrap_or(true);
    on && signed_in()
}

/// The sign-in state file every browser restores (the sign-in window's own).
pub(crate) fn sign_in_state_file() -> Option<PathBuf> {
    Some(state_dir()?.join(format!("{SIGN_IN}-{SIGN_IN}.json")))
}

/// Forgets every sign-in: the next tasks start signed out. An open sign-in
/// window closes first: it saves its state as it closes, which would bring
/// the logins back.
pub fn forget_sign_in() -> Result<()> {
    if let Some(path) = executable() {
        let _ = call(&path, SIGN_IN, &["close"]);
    }
    accounts::forget_import();
    let Some(dir) = state_dir() else {
        return Ok(());
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if is_sign_in_state(&entry.file_name().to_string_lossy()) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// agent-browser's saved state for Agent-Desk's namespace:
/// `~/.agent-browser/namespaces/hex/state/sessions/<key>-<session>.json`.
fn state_dir() -> Option<PathBuf> {
    Some(
        crate::app_paths::home_dir()?
            .join(".agent-browser/namespaces")
            .join(NAMESPACE)
            .join("state/sessions"),
    )
}

fn is_sign_in_state(file: &str) -> bool {
    file.starts_with(&format!("{SIGN_IN}-")) && file.ends_with(".json")
}

/// The saved sign-ins are session tokens in plain JSON: only this user may
/// read the folder.
fn private_state() {
    #[cfg(unix)]
    if let Some(dir) = state_dir() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
}

/// agent-browser with Agent-Desk's agent PATH (and Lightpanda's folder) and none
/// of the person's own `AGENT_BROWSER_*` settings.
fn base_command(path: &Path) -> Command {
    let mut command = search::command(path);
    let extra: Vec<PathBuf> = [Some(path.to_owned()), lightpanda()]
        .into_iter()
        .flatten()
        .collect();
    command
        .env_clear()
        .envs(backends::agent_env().into_iter().filter(|(key, _)| {
            !key.eq_ignore_ascii_case("PATH") && !key.starts_with("AGENT_BROWSER_")
        }));
    command.env("PATH", search::path_value(&extra));
    command
}

/// agent-browser for `session` in Agent-Desk's namespace, headless and clean; a
/// signed-in browser asks for the sign-ins per call ([`sessions`]). The
/// sign-in window's own session loads and saves them.
pub(crate) fn command(path: &Path, session: &str) -> Command {
    let mut command = base_command(path);
    command.envs(session_env(session, config().as_deref()));
    command
}

/// The environment that pins agent-browser to `session`.
pub fn session_env(session: &str, config: Option<&Path>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = [
        ("AGENT_BROWSER_NAMESPACE", NAMESPACE),
        ("AGENT_BROWSER_SESSION", session),
        ("AGENT_BROWSER_IDLE_TIMEOUT_MS", IDLE_MS),
        // Saved sign-ins otherwise lapse after 30 days.
        ("AGENT_BROWSER_STATE_EXPIRE_DAYS", "365"),
        // Lightpanda says it is an old Chrome, which agent-browser would
        // otherwise warn about in every reply.
        ("AGENT_BROWSER_NO_VERSION_CHECK", "1"),
        ("LIGHTPANDA_DISABLE_TELEMETRY", "true"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    if session == SIGN_IN {
        env.push(("AGENT_BROWSER_RESTORE".into(), SIGN_IN.into()));
        env.push(("AGENT_BROWSER_RESTORE_SAVE".into(), "always".into()));
    }
    if let Some(config) = config {
        env.push((
            "AGENT_BROWSER_CONFIG".into(),
            config.to_string_lossy().into_owned(),
        ));
    }
    env
}

/// Agent-Desk's agent-browser config, in place of the person's
/// `~/.agent-browser/config.json` (which could make every browser visible
/// or open their own Chrome profile).
fn config() -> Option<PathBuf> {
    let path = crate::assistant::store::assistant_dir()
        .ok()?
        .join("agent-browser.json");
    let wanted = br#"{"headed": false}"#;
    if std::fs::read(&path).ok().as_deref() != Some(wanted.as_slice()) {
        std::fs::write(&path, wanted).ok()?;
    }
    Some(path)
}

/// Runs one agent-browser command for `session` with `--json`, and reads
/// its reply's `data`.
pub(crate) fn call(path: &Path, session: &str, args: &[&str]) -> Result<Value, String> {
    let mut command = command(path, session);
    command.arg("--json").args(args);
    let ran = run::bounded(command, COMMAND_LIMIT).map_err(|error| error.to_string())?;
    if ran.timed_out() {
        return Err(format!(
            "`agent-browser {}` took more than {} s",
            args.join(" "),
            COMMAND_LIMIT.as_secs()
        ));
    }
    reply(&ran.stdout).unwrap_or_else(|| {
        let words = ran.last_words();
        Err(if words.is_empty() {
            "agent-browser said nothing".into()
        } else {
            words
        })
    })
}

/// A `--json` reply: `{"success": true, "data": …}` or `{"success": false,
/// "error": "…"}` (the last line that is one).
pub fn reply(output: &str) -> Option<Result<Value, String>> {
    let value = last_json(output)?;
    match value.get("success")?.as_bool()? {
        true => Some(Ok(value.get("data").cloned().unwrap_or(Value::Null))),
        false => Some(Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("agent-browser failed")
            .to_owned())),
    }
}

fn last_json(output: &str) -> Option<Value> {
    output
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(Value::is_object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_quick_turn_s_open_browsers_stay_on_the_desk() {
        use crate::assistant::types::Lane;
        assert!(stays_on_desk(Lane::Here, || true));
        assert!(!stays_on_desk(Lane::Here, || false));
        // A job (escalated from a hold or not) closes its research
        // browsers, and never even asks which are open.
        assert!(!stays_on_desk(Lane::Background, || panic!("not asked")));
    }

    #[test]
    fn a_browser_started_ahead_is_the_one_its_first_call_would_start() {
        let with_accounts = sessions::Sessions::new("t", Engine::Lightpanda)
            .with_accounts(true)
            .default_launch();
        let (args, env) = prewarm_command(&with_accounts);
        // It reads the address: a browser already in use keeps its page.
        assert_eq!(args, ["--engine", "lightpanda", "get", "url"]);
        assert_eq!(
            env,
            [
                ("AGENT_BROWSER_RESTORE".to_owned(), SIGN_IN.to_owned()),
                ("AGENT_BROWSER_RESTORE_SAVE".to_owned(), "never".to_owned()),
            ]
        );
        let clean = sessions::Sessions::new("t", Engine::Lightpanda).default_launch();
        let (args, env) = prewarm_command(&clean);
        assert_eq!(args, ["--engine", "lightpanda", "get", "url"]);
        assert!(env.is_empty(), "a clean browser restores nothing");
        // The warm Chrome is the one a first `signedIn: true` call gets.
        let mut sessions = sessions::Sessions::new("t", Engine::Lightpanda);
        let (name, chrome) = sessions.warm_chrome();
        let (args, env) = prewarm_command(&chrome);
        assert_eq!(args, ["--engine", "chrome", "get", "url"]);
        assert_eq!(env[0].1, SIGN_IN);
        let mut first = serde_json::json!({"session": "mail", "signedIn": true})
            .as_object()
            .cloned()
            .unwrap();
        sessions.pin("agent_browser_open", &mut first).unwrap();
        assert_eq!(first["session"], name.as_str());
    }

    fn markers(name: &str) -> (Used, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("agent-desk-used-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Used(dir.clone()), dir)
    }

    /// Only browsers the task used show as minis: a warm one it never
    /// touched is left out of the listing, and closing forgets the use.
    #[test]
    fn only_used_browsers_show() {
        let (used, dir) = markers("show");
        let main = session("t1");
        let warm = sessions::full_name("t1", "_chrome");
        let listed = vec![main.clone(), warm.clone()];
        assert!(used.only_used(listed.clone()).is_empty());
        used.mark(&main);
        assert_eq!(used.only_used(listed.clone()), std::slice::from_ref(&main));
        used.mark(&warm);
        assert_eq!(used.only_used(listed.clone()), listed);
        used.forget(&warm);
        assert!(!used.has(&warm) && used.has(&main));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A task's end closes its unused browsers even when its used ones
    /// stay on the desk.
    #[test]
    fn unused_browsers_close_with_their_task() {
        let main = session("t1");
        let warm = sessions::full_name("t1", "_chrome");
        let (kept, closed) =
            split_used(vec![main.clone(), warm.clone()], |session| session == main);
        assert_eq!(kept, std::slice::from_ref(&main));
        assert_eq!(closed, [warm]);
        // Only a used one keeps a quick turn's browsers on the desk.
        assert!(!stays_on_desk(crate::assistant::types::Lane::Here, || {
            false
        }));
    }

    /// A warm start shows no restored page: it goes blank unless its task
    /// used it already.
    #[test]
    fn a_warm_start_goes_blank_until_used() {
        assert!(blank_needed(Some("https://www.google.com/"), false));
        assert!(!blank_needed(Some("https://www.google.com/"), true));
        assert!(!blank_needed(Some("about:blank"), false));
        assert!(!blank_needed(Some(""), false));
        assert!(!blank_needed(None, false));
    }

    #[test]
    fn only_jobs_naming_a_site_the_person_signed_in_to_warm_a_chrome() {
        let sites: Vec<String> = [
            ".example-mail.com",
            "accounts.workspace.example.org",
            "shop.kiosk.co.uk",
            "x.com",
        ]
        .iter()
        .map(|site| (*site).to_owned())
        .collect();
        // The site's own name, or its address.
        assert!(names_site("Schreib 20 Entwürfe in Example-Mail", &sites));
        assert!(names_site("Schau in Example nach", &sites));
        assert!(names_site("Bestell bei Kiosk", &sites));
        assert!(names_site("open accounts.workspace.example.org", &sites));
        // Not a site they signed in to, a part of a word, or too short a name.
        assert!(!names_site(
            "Recherchiere 10 Lieferanten für Keramik",
            &sites
        ));
        assert!(!names_site("Kiosks in Berlin", &sites));
        assert!(!names_site("Post it on X", &sites));
        assert!(!names_site("Schreib in Example-Mail", &[]));
        assert!(!names_site("", &sites));
        // A site's name that is a word of its own (search) starts nothing;
        // its address, or the name as a place to go, does.
        let google = vec![".google.com".to_owned(), "mail.google.com".to_owned()];
        assert!(!names_site(
            "Google mal, wie das Wetter morgen wird",
            &google
        ));
        assert!(!names_site("google the opening hours of the zoo", &google));
        assert!(!names_site("Was sagt Google zu dem Fehler?", &google));
        assert!(names_site("öffne google.com", &google));
        assert!(names_site("check mail.google.com", &google));
        assert!(names_site("schau auf google.de nach", &google));
        assert!(names_site("Log dich in Google ein", &google));
        assert!(names_site("my Google account settings", &google));
        assert_eq!(site_name("mail.example.com"), Some("example"));
        assert_eq!(site_name("example.co.uk"), Some("example"));
        assert_eq!(site_name("localhost"), None);
    }

    #[test]
    fn sign_in_addresses_open_the_selected_website_only() {
        assert_eq!(
            sign_in_url(" mail.google.com ").unwrap(),
            "https://mail.google.com/"
        );
        assert_eq!(
            sign_in_url("https://example.org/login").unwrap(),
            "https://example.org/login"
        );
        for invalid in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:secret@example.org",
            "https://",
        ] {
            assert!(sign_in_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn saved_sites_describe_storage_not_a_blank_state_file_or_a_login_verdict() {
        assert!(sites_in_state(&serde_json::json!({"cookies": [], "origins": []})).is_empty());
        let state = serde_json::json!({
            "cookies": [
                {"domain": ".example.org", "value": "fixture-token"},
                {"domain": "example.org", "value": "another-fixture"},
                {"domain": "empty.example.org", "value": ""}
            ],
            "origins": [
                {"origin": "https://portal.example.org", "localStorage": [{"name": "fixture", "value": "fixture"}]},
                {"origin": "https://blank.example.org", "localStorage": []}
            ]
        });
        assert_eq!(
            sites_in_state(&state),
            ["example.org", "portal.example.org"]
        );
    }

    #[test]
    fn replies_are_read_from_the_last_envelope() {
        let opened = r#"{"success":true,"data":{"lifecycle":{"launched":true},"targetId":"F6","title":"Example Domain","url":"https://example.com/"},"error":null}"#;
        let data = reply(&format!("⚠ a warning\n{opened}\n")).unwrap().unwrap();
        assert_eq!(data["title"], "Example Domain");
        let failed = r#"{"success":false,"data":null,"error":"Element not found: #nope."}"#;
        assert_eq!(reply(failed), Some(Err("Element not found: #nope.".into())));
        let invalid = r#"{"error":"Invalid session name 'a b'.","success":false,"type":"invalid_session_name"}"#;
        assert_eq!(
            reply(invalid),
            Some(Err("Invalid session name 'a b'.".into()))
        );
        assert_eq!(reply("✓ Browser closed"), None);
    }

    #[test]
    fn doctor_says_whether_chrome_is_installed() {
        let ready = r#"{"checks":[{"category":"Environment","id":"env.version","message":"CLI version 0.42.0","status":"pass"},{"category":"Chrome","id":"chrome.installed","message":"Google Chrome for Testing 157","status":"pass"}],"success":true}"#;
        let missing = r#"{"checks":[{"category":"Chrome","id":"chrome.installed","message":"Chrome not found","status":"fail","fix":"agent-browser install"}],"success":false}"#;
        assert_eq!(chrome_installed(ready), Some(true));
        assert_eq!(chrome_installed(missing), Some(false));
        assert_eq!(chrome_installed("not json"), None);
    }

    #[test]
    fn each_task_gets_a_valid_session_of_its_own() {
        assert_eq!(
            session("3f2a9c1e-0b7d-4e55-9a10-6c2f8e4d1b00"),
            "hex-3f2a9c1e-0b7d-4e55-9a10-6c2f8e4d1b00"
        );
        assert_eq!(session("a b/c"), "hex-a-b-c");
        assert!(session(&"x".repeat(200)).len() <= 68);
    }

    #[test]
    fn task_browsers_start_clean_and_only_the_sign_in_window_saves() {
        let env = |session| {
            session_env(session, Some(Path::new("/tmp/hex.json")))
                .into_iter()
                .collect::<std::collections::HashMap<_, _>>()
        };
        let task = env("hex-t1");
        assert_eq!(task["AGENT_BROWSER_SESSION"], "hex-t1");
        assert_eq!(task["AGENT_BROWSER_NAMESPACE"], "hex");
        assert!(!task.contains_key("AGENT_BROWSER_RESTORE"));
        assert_eq!(task["LIGHTPANDA_DISABLE_TELEMETRY"], "true");
        assert_eq!(task["AGENT_BROWSER_CONFIG"], "/tmp/hex.json");
        assert_eq!(env(SIGN_IN)["AGENT_BROWSER_RESTORE"], SIGN_IN);
        assert_eq!(env(SIGN_IN)["AGENT_BROWSER_RESTORE_SAVE"], "always");
        assert!(is_sign_in_state("hex-sign-in-hex-sign-in.json"));
        assert!(!is_sign_in_state("other-hex-t1.json"));
    }

    #[test]
    fn the_server_is_hex_itself_for_the_task() {
        let server = server(
            PathBuf::from("/Applications/Agent-Desk.app/Contents/MacOS/agent-desk"),
            "t1",
            Path::new("/opt/homebrew/bin/agent-browser"),
        );
        assert_eq!(server.args, ["assistant", "browser-mcp", "--task", "t1"]);
        assert!(
            server
                .path_env
                .split(':')
                .any(|dir| dir == "/opt/homebrew/bin")
        );
        let claude: Value = serde_json::from_str(&server.claude_config_as(MCP_NAME)).unwrap();
        assert_eq!(
            claude["mcpServers"]["agent-browser"]["command"],
            "/Applications/Agent-Desk.app/Contents/MacOS/agent-desk"
        );
        assert_eq!(
            server.codex_args_as(MCP_NAME, &["agent_browser_open"])[1],
            r#"mcp_servers.agent-browser.command="/Applications/Agent-Desk.app/Contents/MacOS/agent-desk""#
        );
        assert_eq!(
            server.opencode_config_as(MCP_NAME)["agent-browser"]["command"][4],
            "t1"
        );
    }
}
