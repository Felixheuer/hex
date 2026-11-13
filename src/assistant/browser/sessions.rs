//! A task's browsers: its main one and more it names ("a", "b",
//! "suppliers-2"), each an agent-browser session of its own in Agent-Desk's
//! namespace (`hex-<task>`, `hex-<task>_<name>`), so one job can read many
//! pages at once. How many is the computer's (`capacity::limits`): up to
//! 32 on a big Mac, fewer of them real Chrome.
//!
//! Each starts as one of two kinds and stays that way:
//! - **clean** (the default) for reading and research: a light Lightpanda
//!   browser when it's installed (about a tenth of Chrome's memory), else a
//!   clean Chrome;
//! - **signed in** (`signedIn: true` on its first call) for web apps such as
//!   Gmail or Google Sheets: real Chrome, starting from the sites the person
//!   signed in to for Agent-Desk (never saved back).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::SIGN_IN;

/// Browsers one task may have open at once on this computer, its main one
/// included.
pub fn max_sessions() -> usize {
    crate::assistant::capacity::limits().browsers
}

/// Arguments no task passes through to agent-browser: they would show a
/// window, reach another task's browsers, keep state, or pass raw flags.
/// `session` and `signedIn` are Agent-Desk's own and read here.
pub const WITHHELD_ARGUMENTS: &[&str] = &[
    "namespace",
    "all",
    "headed",
    "extraArgs",
    "restore",
    "restoreSave",
    "restoreCheckUrl",
    "restoreCheckText",
    "restoreCheckFn",
    "capture",
    "caCert",
    "useSystemCa",
    "clearCaCert",
    "idleTimeout",
];

/// Arguments that name where agent-browser writes a file (a screenshot's
/// `path` and `screenshotDir`). agent-browser writes wherever they say,
/// outside any job's sandbox: a page's words could make an agent
/// overwrite the person's Chrome cookies or Agent-Desk's sign-in state. So
/// they may only name an image inside Agent-Desk's own folders
/// ([`output_roots`]).
const OUTPUT_ARGUMENTS: &[&str] = &["path", "screenshotDir"];

/// Where a task's browser may write files: Agent-Desk's folder in
/// Documents (every job's workspace and project lives there) and the temp
/// folder.
fn output_roots() -> Vec<std::path::PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    roots.extend(crate::assistant::agent::workspace::root());
    roots
        .into_iter()
        .filter_map(|root| root.canonicalize().ok().or(Some(root)))
        .collect()
}

/// Whether a task's browser may write `path` (an image file when `image`,
/// else a folder): absolute, inside one of `roots` once its folder's links
/// are followed, not itself a link, and (a file) an image by its name.
fn safe_output(path: &str, image: bool, roots: &[std::path::PathBuf]) -> Result<(), String> {
    use std::path::{Component, Path};
    let refused = || {
        Err(format!(
            "The browser saves files only inside the job's folder (in Documents › Agent-Desk) or the temp folder{}: “{path}” isn't allowed. Leave the path out to get the screenshot back.",
            if image {
                ", as .png, .jpg or .webp"
            } else {
                ""
            }
        ))
    };
    let wanted = Path::new(path.trim());
    if !wanted.is_absolute()
        || wanted
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return refused();
    }
    if image {
        let extension = wanted
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp")) {
            return refused();
        }
        if wanted
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return refused();
        }
    }
    // The nearest folder that exists, with its links followed.
    let mut folder = if image { wanted.parent() } else { Some(wanted) };
    let real = loop {
        let Some(candidate) = folder else {
            return refused();
        };
        if let Ok(real) = candidate.canonicalize() {
            break real;
        }
        folder = candidate.parent();
    };
    if roots.iter().any(|root| real.starts_with(root)) {
        Ok(())
    } else {
        refused()
    }
}

/// The tool that lists a task's browsers, answered by Agent-Desk.
pub const LIST_TOOL: &str = "hex_browser_sessions";

/// The session a task's first signed-in Chrome runs in, whatever the agent
/// calls it: `browser::prewarm_for` may have started it already. A leading
/// `_` is no name an agent may give.
const WARM_CHROME: &str = "_chrome";

/// How a browser started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Clean,
    /// Real Chrome with the person's accounts (`signedIn: true`).
    SignedIn,
    /// The light engine with the person's accounts: every browser's
    /// default while they have some (`browser_signed_in_by_default`).
    Accounts,
}

/// The browser engine behind a clean session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    Chrome,
    Lightpanda,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Lightpanda => "lightpanda",
        }
    }
}

/// The agent-browser session for `name` of `task` ("" is its main one).
pub fn full_name(task: &str, name: &str) -> String {
    let main = super::session(task);
    if name.is_empty() {
        main
    } else {
        format!("{main}_{name}")
    }
}

/// Whether `session` is one of `task`'s.
pub fn belongs(task: &str, session: &str) -> bool {
    let main = super::session(task);
    session == main
        || session
            .strip_prefix(&main)
            .is_some_and(|rest| rest.starts_with('_'))
}

/// How many helpers of `task`'s lead are working, when `task` is a helper.
fn team_working(task: &str) -> Option<usize> {
    let lead = crate::assistant::agent::team::lead_of(task)?;
    let root = crate::assistant::agent::team::root()?;
    let working = crate::assistant::agent::team::members(&root, &lead)
        .iter()
        .filter(|member| !member.state.ended())
        .count();
    Some(working.max(1))
}

fn critically_tight() -> bool {
    crate::assistant::capacity::pressure() == crate::assistant::capacity::Pressure::Critical
}

/// A task's browsers as its MCP server has them.
#[derive(Clone, Debug)]
pub struct Sessions {
    task: String,
    clean: Engine,
    /// A browser that asks for nothing starts with the person's accounts.
    accounts: bool,
    open: BTreeMap<String, Kind>,
    /// Browsers that run under another session than their name: the first
    /// signed-in Chrome, in [`WARM_CHROME`].
    runs_as: BTreeMap<String, String>,
    /// Whether memory is critically tight now: no browser beyond the first
    /// opens (`capacity::pressure`).
    tight: fn() -> bool,
    /// Browsers open at once, at most, and real Chrome ones among them.
    max: usize,
    max_chrome: usize,
    /// Browsers past the task's share of Chrome, run in Lightpanda (no
    /// picture, but light): only when it's installed.
    overflow: std::collections::BTreeSet<String>,
    light_available: bool,
}

impl Sessions {
    pub fn new(task: &str, clean: Engine) -> Self {
        let limits = crate::assistant::capacity::limits();
        // A helper shares the computer with its team (`capacity::share`).
        let limits = team_working(task).map_or(limits, |helpers| {
            crate::assistant::capacity::share(limits, helpers)
        });
        Self {
            task: task.to_owned(),
            clean,
            accounts: false,
            open: BTreeMap::new(),
            runs_as: BTreeMap::new(),
            tight: critically_tight,
            max: limits.browsers,
            max_chrome: limits.chrome,
            overflow: std::collections::BTreeSet::new(),
            light_available: super::lightpanda().is_some(),
        }
    }

    /// Other limits than this computer's (tests).
    #[cfg(test)]
    pub fn with_limits(mut self, max: usize, max_chrome: usize) -> Self {
        self.max = max;
        self.max_chrome = max_chrome;
        self
    }

    /// The engine browser `name` (of `kind`) runs in.
    fn engine_for(&self, name: &str, kind: Kind) -> Engine {
        if self.overflow.contains(name) {
            Engine::Lightpanda
        } else {
            self.engine_of(kind)
        }
    }

    fn engine_of(&self, kind: Kind) -> Engine {
        match kind {
            Kind::SignedIn => Engine::Chrome,
            Kind::Clean | Kind::Accounts => self.clean,
        }
    }

    fn names(&self) -> String {
        self.open
            .keys()
            .map(|name| shown(name))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether Lightpanda is there for the overflow (tests).
    #[cfg(test)]
    pub fn with_light(mut self, available: bool) -> Self {
        self.light_available = available;
        self
    }

    /// How it tells memory is critically tight (tests).
    #[cfg(test)]
    pub fn with_tight(mut self, tight: fn() -> bool) -> Self {
        self.tight = tight;
        self
    }

    /// Browsers that ask for nothing start with the person's accounts.
    pub fn with_accounts(mut self, accounts: bool) -> Self {
        self.accounts = accounts;
        self
    }

    pub fn task(&self) -> &str {
        &self.task
    }

    /// A tool call's arguments pinned to one of this task's browsers;
    /// `Err` is the tool error to answer with instead of calling.
    pub fn pin(&mut self, tool: &str, arguments: &mut Map<String, Value>) -> Result<(), String> {
        for name in WITHHELD_ARGUMENTS {
            arguments.remove(*name);
        }
        for name in OUTPUT_ARGUMENTS {
            if let Some(value) = arguments.get(*name) {
                let image = *name == "path";
                safe_output(value.as_str().unwrap_or_default(), image, &output_roots())?;
            }
        }
        let name = parse_name(arguments.remove("session"))?;
        let signed_in = arguments
            .remove("signedIn")
            .and_then(|value| value.as_bool());
        if tool == "agent_browser_close" {
            arguments.insert("session".into(), self.session_of(&name).into());
            self.open.remove(&name);
            self.runs_as.remove(&name);
            self.overflow.remove(&name);
            return Ok(());
        }
        let wanted = signed_in.map(|on| if on { Kind::SignedIn } else { Kind::Clean });
        let kind = match (self.open.get(&name), wanted) {
            // A browser started with the accounts by default is what a later
            // `signedIn: true` would get in the light engine: keep it.
            (Some(Kind::Accounts), Some(Kind::SignedIn)) => Kind::Accounts,
            (Some(kind), Some(wanted)) if *kind != wanted => {
                return Err(format!(
                    "Browser “{}” started {}; use another session name for a {} one",
                    shown(&name),
                    describe(*kind),
                    describe(wanted)
                ));
            }
            (Some(kind), _) => *kind,
            (None, wanted) => {
                if self.open.len() >= self.max {
                    return Err(format!(
                        "This task already has {} browsers, the most this computer runs \
                         well for one task (or its share beside the other helpers) ({}). Close one (agent_browser_close with its \
                         session) before opening another",
                        self.max,
                        self.names()
                    ));
                }
                // Fifty jobs' browsers can run a computer out of memory:
                // while it's critically tight, a task works with what it has.
                if !self.open.is_empty() && (self.tight)() {
                    return Err(format!(
                        "Memory on this computer is critically tight, so no more browsers open \
                         now. Go on with the ones you have ({}), close one you're done with \
                         (agent_browser_close), or wait a minute and try again",
                        self.names()
                    ));
                }
                let kind = wanted.unwrap_or(if self.accounts {
                    Kind::Accounts
                } else {
                    Kind::Clean
                });
                let chrome = self
                    .open
                    .iter()
                    .filter(|(name, kind)| self.engine_for(name, **kind) == Engine::Chrome)
                    .count();
                let at_share = self.engine_of(kind) == Engine::Chrome && chrome >= self.max_chrome;
                if at_share && kind != Kind::SignedIn && self.light_available {
                    // Past the share: light, without a picture.
                    self.overflow.insert(name.clone());
                } else if at_share {
                    return Err(format!(
                        "This task already has {} real Chrome browsers, the most this computer \
                         runs well for one task. Read pages in light browsers (omit signedIn), \
                         or close a Chrome one you're done with (agent_browser_close)",
                        self.max_chrome
                    ));
                }
                // The first signed-in Chrome is the one started ahead.
                if kind == Kind::SignedIn && !self.runs_as.values().any(|name| name == WARM_CHROME)
                {
                    self.runs_as.insert(name.clone(), WARM_CHROME.to_owned());
                }
                self.open.insert(name.clone(), kind);
                kind
            }
        };
        arguments.insert("session".into(), self.session_of(&name).into());
        arguments.extend(launch_arguments(kind, self.engine_for(&name, kind)));
        Ok(())
    }

    /// The agent-browser session browser `name` runs in.
    fn session_of(&self, name: &str) -> String {
        full_name(
            &self.task,
            self.runs_as.get(name).map_or(name, String::as_str),
        )
    }

    /// The session and launch arguments of the task's first signed-in
    /// Chrome, to start it ahead (`browser::prewarm_for`).
    pub fn warm_chrome(&self) -> (String, Map<String, Value>) {
        (
            full_name(&self.task, WARM_CHROME),
            launch_arguments(Kind::SignedIn, Engine::Chrome),
        )
    }

    /// The `restore` and engine arguments a browser that asks for nothing
    /// would start with (`browser::prewarm_for`, which starts a job's main
    /// browser before it asks, the same way).
    pub fn default_launch(&self) -> Map<String, Value> {
        let kind = if self.accounts {
            Kind::Accounts
        } else {
            Kind::Clean
        };
        launch_arguments(kind, self.engine_of(kind))
    }

    /// The task's browsers for [`LIST_TOOL`]; `running` are the sessions
    /// agent-browser has open.
    pub fn list(&self, running: &[String]) -> Value {
        let browsers: Vec<Value> = self
            .open
            .iter()
            .map(|(name, kind)| {
                let engine = self.engine_for(name, *kind);
                json!({
                    "session": shown(name),
                    "signedIn": *kind != Kind::Clean,
                    "engine": engine.name(),
                    "running": running.contains(&self.session_of(name)),
                })
            })
            .collect();
        json!({ "browsers": browsers, "max": self.max, "maxChrome": self.max_chrome })
    }
}

/// What a browser of `kind` on `engine` starts with: the person's accounts
/// (never saved back) unless it's clean, and its engine.
fn launch_arguments(kind: Kind, engine: Engine) -> Map<String, Value> {
    let mut arguments = Map::new();
    if kind != Kind::Clean {
        arguments.insert("restore".into(), SIGN_IN.into());
        arguments.insert("restoreSave".into(), "never".into());
    }
    let mut extra = vec![json!("--engine"), json!(engine.name())];
    // Real Chrome sessions run the person's Chromium-family browser where
    // there is one, so pages with H.264 video play (`media::codecs`).
    if engine == Engine::Chrome
        && let Some(executable) = crate::assistant::media::codecs::chromium_executable()
    {
        extra.extend([json!("--executable-path"), json!(executable)]);
    }
    arguments.insert("extraArgs".into(), Value::Array(extra));
    arguments
}

fn shown(name: &str) -> &str {
    if name.is_empty() { "main" } else { name }
}

fn describe(kind: Kind) -> &'static str {
    match kind {
        Kind::Clean => "clean",
        Kind::SignedIn => "signed in",
        Kind::Accounts => "light with the person's accounts",
    }
}

/// A `session` argument: absent, "", or "main" is the main browser; else 1
/// to 16 letters, digits, `-`, or `_`, lowercased.
fn parse_name(value: Option<Value>) -> Result<String, String> {
    let text = match value {
        None | Some(Value::Null) => return Ok(String::new()),
        Some(Value::String(text)) => text.trim().to_lowercase(),
        Some(other) => return Err(format!("session must be a short name, not {other}")),
    };
    if text.is_empty() || text == "main" {
        return Ok(String::new());
    }
    let valid = text.chars().count() <= 16
        // A leading `_` is Agent-Desk's own ([`WARM_CHROME`]).
        && !text.starts_with('_')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(text)
    } else {
        Err(format!(
            "session “{text}” isn't a valid name: use up to 16 letters, digits, - or _"
        ))
    }
}

/// The `session` and `signedIn` arguments Agent-Desk adds to every tool's schema,
/// and the list tool.
pub fn describe_tools(result: &mut Value) {
    let last_page = result.get("nextCursor").is_none_or(Value::is_null);
    let Some(tools) = result.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools.iter_mut() {
        let opens = matches!(
            tool.get("name").and_then(Value::as_str),
            Some("agent_browser_open" | "agent_browser_tab_new")
        );
        let Some(schema) = tool.get_mut("inputSchema") else {
            continue;
        };
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for name in WITHHELD_ARGUMENTS {
                properties.remove(*name);
            }
            properties.insert(
                "session".into(),
                json!({"type": "string", "description": format!(
                    "Which of this task's browsers: omit for the main one, or a short name \
                     (a, b, … or up to 16 letters, digits, - and _) for another one that runs \
                     beside it. Up to {} per task.", max_sessions())}),
            );
            if opens {
                properties.insert(
                    "signedIn".into(),
                    json!({"type": "boolean", "description":
                        "On a browser's first call only: true starts it in real Chrome with \
                         the person's accounts (for heavy web apps and logins); \
                         false starts a clean browser with no accounts. Omitted: a light \
                         browser, with the person's accounts when they have some."}),
                );
            }
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| {
                name.as_str()
                    .is_none_or(|name| !WITHHELD_ARGUMENTS.contains(&name) && name != "session")
            });
        }
    }
    if last_page {
        tools.push(json!({
            "name": LIST_TOOL,
            "title": "This task's browsers",
            "description": "List this task's browsers: each one's session name, whether it \
                started signed in, its engine, and whether it is running. Cheap; touches no page.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_browser_writes_files_only_inside_agent_desks_folders() {
        let base = std::env::temp_dir().join(format!("hex-out-{}", std::process::id()));
        let root = base.join("Agent-Desk");
        let job = root.join("2026-11-13 Job");
        let outside = base.join("Chrome/Default");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let roots = vec![root.canonicalize().unwrap()];
        let ok = |path: &std::path::Path, image| {
            super::safe_output(path.to_str().unwrap(), image, &roots).is_ok()
        };
        assert!(ok(&job.join("sent.png"), true));
        assert!(
            ok(&job.join("new/sub/shot.jpg"), true),
            "a folder still to be made"
        );
        assert!(ok(&job, false));
        assert!(
            !ok(&outside.join("Cookies"), true),
            "not an image, not inside"
        );
        assert!(!ok(&outside.join("shot.png"), true));
        assert!(!ok(&job.join("../../Chrome/Default/x.png"), true));
        assert!(!ok(&job.join("state.json"), true));
        assert!(!ok(std::path::Path::new("relative.png"), true));
        #[cfg(unix)]
        {
            // A link inside that points out.
            std::os::unix::fs::symlink(&outside, job.join("escape")).unwrap();
            assert!(!ok(&job.join("escape/shot.png"), true));
            std::os::unix::fs::symlink(outside.join("Cookies"), job.join("cookie.png")).unwrap();
            assert!(!ok(&job.join("cookie.png"), true));
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    use super::*;

    #[test]
    fn critically_tight_memory_opens_no_browser_beyond_the_first() {
        let mut sessions = Sessions::new("t1", Engine::Chrome).with_tight(|| true);
        let args = |session: &str| {
            let mut map = Map::new();
            map.insert("session".into(), session.into());
            map
        };
        assert!(
            sessions
                .pin("agent_browser_open", &mut args("main"))
                .is_ok(),
            "one always"
        );
        assert!(
            sessions
                .pin("agent_browser_snapshot", &mut args("main"))
                .is_ok(),
            "used again"
        );
        let refused = sessions
            .pin("agent_browser_open", &mut args("b"))
            .unwrap_err();
        assert!(refused.contains("critically tight"), "{refused}");
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn the_main_browser_is_clean_and_light_by_default() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda);
        let mut call = args(json!({"url": "https://example.com", "headed": true,
            "extraArgs": ["--headed"], "namespace": "x", "restore": true}));
        sessions.pin("agent_browser_open", &mut call).unwrap();
        assert_eq!(
            Value::Object(call),
            json!({"url": "https://example.com", "session": "hex-t1",
                "extraArgs": ["--engine", "lightpanda"]})
        );
    }

    #[test]
    fn a_named_signed_in_browser_runs_chrome_from_the_sign_ins() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda);
        let mut call =
            args(json!({"session": "Mail", "signedIn": true, "url": "https://mail.google.com"}));
        sessions.pin("agent_browser_open", &mut call).unwrap();
        // The first signed-in Chrome runs in the session started ahead.
        assert_eq!(call["session"], "hex-t1__chrome");
        assert_eq!(call["restore"], SIGN_IN);
        assert_eq!(call["restoreSave"], "never");
        assert_eq!(call["extraArgs"], json!(["--engine", "chrome"]));
        // Later calls keep its kind without saying it again.
        let mut later = args(json!({"session": "mail"}));
        sessions.pin("agent_browser_snapshot", &mut later).unwrap();
        assert_eq!(later["restore"], SIGN_IN);
        // It can't turn into a clean one.
        let mut clean = args(json!({"session": "mail", "signedIn": false}));
        assert!(sessions.pin("agent_browser_open", &mut clean).is_err());
    }

    #[test]
    fn a_task_has_at_most_its_computer_s_browsers_and_closing_frees_one() {
        let mut sessions = Sessions::new("t1", Engine::Chrome).with_limits(12, 12);
        for name in [
            "main", "a", "b", "c", "d", "e", "f", "g", "i", "j", "k", "l",
        ] {
            sessions
                .pin("agent_browser_open", &mut args(json!({"session": name})))
                .unwrap();
        }
        let refused = sessions.pin("agent_browser_open", &mut args(json!({"session": "h"})));
        assert!(refused.unwrap_err().contains("already has 12 browsers"));
        let mut close = args(json!({"session": "c", "all": true}));
        sessions.pin("agent_browser_close", &mut close).unwrap();
        assert_eq!(Value::Object(close), json!({"session": "hex-t1_c"}));
        sessions
            .pin("agent_browser_open", &mut args(json!({"session": "h"})))
            .unwrap();
    }

    #[test]
    fn browsers_are_chrome_with_pictures_and_only_the_overflow_is_light() {
        let mut sessions = Sessions::new("t1", Engine::Chrome)
            .with_limits(32, 2)
            .with_light(true);
        let engine = |sessions: &mut Sessions, name: &str| {
            let mut call = args(json!({"session": name}));
            sessions.pin("agent_browser_open", &mut call).unwrap();
            call["extraArgs"][1].as_str().unwrap().to_owned()
        };
        assert_eq!(engine(&mut sessions, "a"), "chrome");
        assert_eq!(engine(&mut sessions, "b"), "chrome");
        assert_eq!(engine(&mut sessions, "c"), "lightpanda", "past the share");
        // Without Lightpanda the share is a hard limit.
        let mut without = Sessions::new("t2", Engine::Chrome)
            .with_limits(32, 1)
            .with_light(false);
        assert_eq!(engine(&mut without, "a"), "chrome");
        let mut call = args(json!({"session": "b"}));
        assert!(without.pin("agent_browser_open", &mut call).is_err());
    }

    #[test]
    fn twenty_light_browsers_open_but_real_chrome_is_held_to_its_share() {
        // This Mac's limits: 32 browsers, 16 of them Chrome.
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_limits(32, 16);
        for n in 0..20 {
            let mut call = args(json!({"session": format!("s{n}")}));
            sessions.pin("agent_browser_open", &mut call).unwrap();
            assert_eq!(call["extraArgs"], json!(["--engine", "lightpanda"]));
        }
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_limits(32, 2);
        for name in ["a", "b"] {
            let mut call = args(json!({"session": name, "signedIn": true}));
            sessions.pin("agent_browser_open", &mut call).unwrap();
        }
        let mut third = args(json!({"session": "c", "signedIn": true}));
        let refused = sessions.pin("agent_browser_open", &mut third).unwrap_err();
        assert!(refused.contains("2 real Chrome browsers"), "{refused}");
        // A light one still opens.
        sessions
            .pin("agent_browser_open", &mut args(json!({"session": "c"})))
            .unwrap();
        // A helper's main browser is started ahead the way its first call
        // would start it.
        let light = Sessions::new("t2", Engine::Lightpanda).with_accounts(true);
        let ahead = light.default_launch();
        assert_eq!(ahead["restore"], SIGN_IN);
        assert_eq!(ahead["extraArgs"], json!(["--engine", "lightpanda"]));
        let clean = Sessions::new("t2", Engine::Lightpanda).default_launch();
        assert!(clean.get("restore").is_none());
    }

    #[test]
    fn the_first_signed_in_chrome_runs_in_the_one_started_ahead() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_limits(12, 4);
        let (warm, _) = sessions.warm_chrome();
        let mut gmail = args(json!({"session": "gmail", "signedIn": true}));
        sessions.pin("agent_browser_open", &mut gmail).unwrap();
        assert_eq!(gmail["session"], warm.as_str());
        // Its later calls, and the list, follow it there.
        let mut later = args(json!({"session": "gmail"}));
        sessions.pin("agent_browser_snapshot", &mut later).unwrap();
        assert_eq!(later["session"], warm.as_str());
        assert_eq!(
            sessions.list(std::slice::from_ref(&warm))["browsers"][0],
            json!({"session": "gmail", "signedIn": true, "engine": "chrome", "running": true})
        );
        // A second signed-in Chrome runs under its own name.
        let mut sheets = args(json!({"session": "sheets", "signedIn": true}));
        sessions.pin("agent_browser_open", &mut sheets).unwrap();
        assert_eq!(sheets["session"], "hex-t1_sheets");
        // Closing the first frees the warm one's name for the next.
        let mut close = args(json!({"session": "gmail"}));
        sessions.pin("agent_browser_close", &mut close).unwrap();
        assert_eq!(close["session"], warm.as_str());
        let mut again = args(json!({"session": "mail", "signedIn": true}));
        sessions.pin("agent_browser_open", &mut again).unwrap();
        assert_eq!(again["session"], warm.as_str());
        // Light browsers keep their own names; nobody names Agent-Desk's own.
        let mut light = args(json!({"session": "a"}));
        sessions.pin("agent_browser_open", &mut light).unwrap();
        assert_eq!(light["session"], "hex-t1_a");
        let refused = sessions.pin(
            "agent_browser_open",
            &mut args(json!({"session": "_chrome"})),
        );
        assert!(refused.is_err());
        assert!(belongs("t1", &warm), "closed with the task");
    }

    #[test]
    fn session_names_stay_inside_the_task() {
        let mut sessions = Sessions::new("t1", Engine::Chrome);
        for bad in [json!("../x"), json!("a b"), json!("x".repeat(17)), json!(3)] {
            let refused = sessions.pin("agent_browser_open", &mut args(json!({"session": bad})));
            assert!(refused.is_err(), "{bad}");
        }
        assert!(belongs("t1", "hex-t1"));
        assert!(belongs("t1", "hex-t1_a"));
        assert!(!belongs("t1", "hex-t10"));
        assert!(!belongs("t1", "hex-t1-x"));
    }

    #[test]
    fn the_list_says_what_each_browser_is() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_limits(12, 4);
        sessions
            .pin("agent_browser_open", &mut args(json!({})))
            .unwrap();
        sessions
            .pin(
                "agent_browser_open",
                &mut args(json!({"session": "a", "signedIn": true})),
            )
            .unwrap();
        assert_eq!(
            sessions.list(&["hex-t1".into()]),
            json!({"browsers": [
                {"session": "main", "signedIn": false, "engine": "lightpanda", "running": true},
                {"session": "a", "signedIn": true, "engine": "chrome", "running": false},
            ], "max": 12, "maxChrome": 4})
        );
    }

    #[test]
    fn every_tool_offers_session_and_open_offers_signed_in() {
        let mut result = json!({"tools": [
            {"name": "agent_browser_open", "inputSchema": {"type": "object", "required": ["session"],
                "properties": {"url": {"type": "string"}, "session": {"type": "string"},
                    "headed": {"type": "boolean"}, "restore": {}}}},
            {"name": "agent_browser_close", "inputSchema": {"type": "object",
                "properties": {"all": {"type": "boolean"}}}},
        ]});
        describe_tools(&mut result);
        let tools = result["tools"].as_array().unwrap();
        let open = &tools[0]["inputSchema"];
        let mut keys: Vec<_> = open["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(keys, ["session", "signedIn", "url"]);
        assert_eq!(open["required"], json!([]));
        assert!(tools[1]["inputSchema"]["properties"].get("all").is_none());
        assert!(
            tools[1]["inputSchema"]["properties"]
                .get("signedIn")
                .is_none()
        );
        assert_eq!(tools[2]["name"], LIST_TOOL);
    }

    #[test]
    fn with_the_person_s_accounts_every_browser_starts_signed_in_unless_asked_clean() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_accounts(true);
        // Asking for nothing: the light engine, with the accounts.
        let mut plain = args(json!({"url": "https://github.com"}));
        sessions.pin("agent_browser_open", &mut plain).unwrap();
        assert_eq!(plain["restore"], SIGN_IN);
        assert_eq!(plain["restoreSave"], "never");
        assert_eq!(plain["extraArgs"], json!(["--engine", "lightpanda"]));
        // A later signedIn: true keeps that browser as it started.
        let mut again = args(json!({"signedIn": true}));
        sessions.pin("agent_browser_snapshot", &mut again).unwrap();
        assert_eq!(again["restore"], SIGN_IN);
        // signedIn: true opens real Chrome with the accounts.
        let mut chrome = args(json!({"session": "mail", "signedIn": true}));
        sessions.pin("agent_browser_open", &mut chrome).unwrap();
        assert_eq!(chrome["restore"], SIGN_IN);
        assert_eq!(chrome["extraArgs"], json!(["--engine", "chrome"]));
        // signedIn: false is a clean browser.
        let mut clean = args(json!({"session": "c", "signedIn": false}));
        sessions.pin("agent_browser_open", &mut clean).unwrap();
        assert!(clean.get("restore").is_none());
        let listed = sessions.list(&[]);
        let by_name = |name: &str| {
            listed["browsers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|browser| browser["session"] == name)
                .unwrap()["signedIn"]
                .clone()
        };
        assert_eq!(by_name("main"), true);
        assert_eq!(by_name("c"), false);
    }

    #[test]
    fn without_accounts_a_browser_that_asks_for_nothing_is_clean() {
        let mut sessions = Sessions::new("t1", Engine::Lightpanda).with_accounts(false);
        let mut plain = args(json!({}));
        sessions.pin("agent_browser_open", &mut plain).unwrap();
        assert!(plain.get("restore").is_none());
    }
}
