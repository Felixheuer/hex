//! One task's guard: every tool call is checked against the task's lane and
//! leases, then forwarded to agent-desktop or refused with a tool error the
//! model can act on. Synchronous; `server` runs it off the MCP runtime.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

mod lookup;

use super::child::{self, Desktop, Window};
use super::policy::{self, Kind};
use super::state::{Acquire, LeaseFile, Store};
use crate::assistant::types::{Lane, TaskWindow};

/// What the guard knows about its task.
#[derive(Clone, Debug)]
pub struct Config {
    pub task: String,
    pub lane: Lane,
    pub here_window: Option<TaskWindow>,
    pub physical_input: bool,
    pub blocked_apps: Vec<String>,
}

/// Where a snapshot's refs point.
#[derive(Clone, Debug, PartialEq)]
struct Origin {
    window: Option<String>,
    app: String,
}

/// One window (or an app's window-less surface) a call reads or operates.
#[derive(Clone, Debug)]
struct Resolved {
    id: Option<String>,
    app: String,
    focused: bool,
}

impl Resolved {
    fn of(window: &Window) -> Self {
        Self {
            id: Some(window.id.clone()),
            app: window.app.clone(),
            focused: window.focused,
        }
    }
}

/// A launch just made, whose new windows are the task's.
struct Launched {
    app: String,
    pid: Option<i64>,
}

/// How long an app the task worked in has its new windows adopted.
const WATCH: Duration = Duration::from_secs(120);

pub const RESUMED: &str = "The person used this window while this task was paused; take a fresh \
snapshot before acting. (The action you asked for was not sent.)";

pub struct Guard<D: Desktop> {
    config: Config,
    store: Store,
    pid: u32,
    desktop: D,
    snapshots: HashMap<String, Origin>,
    /// Window ids on screen at the last look (Background: new ones may be
    /// adopted).
    known: Option<HashSet<String>>,
    watched: HashMap<String, Instant>,
    /// This call's `list-windows`, once fetched.
    fresh: Option<Vec<Window>>,
    notes: Vec<String>,
    resumed: Option<String>,
    note_pending: bool,
    here_ready: bool,
    bundle_ids: Option<Vec<(String, String)>>,
    /// How often a paused action looks again.
    pub poll: Duration,
}

impl<D: Desktop> Guard<D> {
    pub fn new(config: Config, store: Store, pid: u32, desktop: D) -> Self {
        let resumed = store.resumed(&config.task);
        Self {
            config,
            store,
            pid,
            desktop,
            snapshots: HashMap::new(),
            known: None,
            watched: HashMap::new(),
            fresh: None,
            notes: Vec::new(),
            resumed,
            note_pending: false,
            here_ready: false,
            bundle_ids: None,
            poll: Duration::from_millis(250),
        }
    }

    pub fn desktop(&mut self) -> &mut D {
        &mut self.desktop
    }

    /// The tools this task gets: agent-desktop's, less those it may never
    /// use, plus `desktop_claim_window` for a Background task.
    pub fn tools(&mut self) -> Result<Vec<Value>, String> {
        let mut tools = self.desktop.tools()?;
        let background = self.config.lane == Lane::Background;
        let physical = self.config.physical_input;
        tools.retain(|tool| {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            match policy::kind(name, &Map::new()) {
                Kind::Refused(_) | Kind::Claim => false,
                Kind::Physical => physical,
                Kind::Launch | Kind::FocusWindow => background == (name == "desktop_launch"),
                _ => true,
            }
        });
        if background {
            tools.push(claim_tool());
        }
        Ok(tools)
    }

    /// The lane's rules, for the server's instructions.
    pub fn rules(&self) -> String {
        let input = if self.config.physical_input {
            "Real pointer and keyboard input is allowed."
        } else {
            "Real pointer and keyboard input (hover, drag, mouse_*, double_click) is not allowed; \
             act through refs."
        };
        match self.config.lane {
            Lane::Here => format!(
                "Hex guards these tools. This task was asked about the window in front ({}); it \
                 may read and act only in that window. Other windows, launching apps, and \
                 blocked apps are refused. {input}",
                self.config
                    .here_window
                    .as_ref()
                    .map_or("unknown".into(), describe_task_window)
            ),
            Lane::Background => format!(
                "Hex guards these tools. This task runs in the background beside the person: it \
                 may read and act only in windows it holds, which are windows it opened with \
                 desktop_launch (and new windows its own actions open) and free windows it took \
                 with desktop_claim_window. Never the window in front of the person. \
                 desktop_list_windows and desktop_list_apps see everything. Pass window_id when \
                 an app has several windows. {input}"
            ),
        }
    }

    /// Checks and forwards one call; the MCP `CallToolResult` to return.
    pub fn call(&mut self, tool: &str, args: Map<String, Value>) -> Value {
        self.fresh = None;
        self.notes.clear();
        self.notice_resume();
        let mut args = args;
        let outcome = self.run(tool, &mut args);
        let forwarded = outcome.is_ok();
        let mut result = match outcome {
            Ok(result) | Err(result) => result,
        };
        if forwarded && self.note_pending {
            self.note_pending = false;
            self.notes
                .push(RESUMED.replace(" (The action you asked for was not sent.)", ""));
        }
        if !self.notes.is_empty()
            && let Some(content) = result.get_mut("content").and_then(Value::as_array_mut)
        {
            for note in self.notes.drain(..) {
                content.push(json!({"type": "text", "text": format!("Hex: {note}")}));
            }
        }
        result
    }

    fn run(&mut self, tool: &str, args: &mut Map<String, Value>) -> Result<Value, Value> {
        let kind = policy::kind(tool, args);
        if let Some(app) = text(args, "app")
            && self.blocked(&app)
        {
            return Err(self.blocked_refusal(tool, &app));
        }
        if self.config.lane == Lane::Here {
            self.ensure_here();
        }
        match kind {
            Kind::Refused(why) => return Err(refusal(tool, "POLICY_DENIED", why)),
            Kind::Claim => return self.claim(tool, args),
            Kind::Free => {
                let result = self.forward(tool, args)?;
                let background = self.config.lane == Lane::Background;
                if background && tool == "desktop_wait" && args.contains_key("event") {
                    let _ = self.list(None);
                }
                if tool == "desktop_list_windows" {
                    self.windows_note();
                }
                return Ok(result);
            }
            _ => {}
        }
        if kind.acts() {
            self.gate(tool)?;
        }
        if kind == Kind::Physical && !self.config.physical_input {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "Real pointer and keyboard input isn't allowed for this task; act through refs \
                 (click, type, set_value, select, scroll, press with app).",
            ));
        }
        if self.config.lane == Lane::Here && matches!(kind, Kind::Launch) {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This task was asked about the window in front and acts only there; opening \
                 apps is for Background tasks.",
            ));
        }
        if kind == Kind::Launch {
            return self.launch(tool, args);
        }
        if kind == Kind::FocusWindow && self.config.lane == Lane::Background {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "Focusing brings a window in front of the person; Background tasks work without \
                 it.",
            ));
        }
        let targets =
            policy::targets(tool, args).map_err(|why| refusal(tool, "POLICY_DENIED", &why))?;
        let mut resolved = Vec::new();
        for target in targets {
            resolved.extend(self.resolve(tool, kind, target, args)?);
        }
        let held = self.held();
        for window in &resolved {
            self.check(tool, kind, window, &held)?;
        }
        if kind.acts() && self.config.lane == Lane::Background {
            for window in &resolved {
                self.watch(&window.app);
            }
        }
        let hands = (kind.acts() && self.config.physical_input)
            .then(|| self.store.hands_on(&self.config.task, self.pid));
        let result = self.forward(tool, args);
        drop(hands);
        let result = result?;
        self.record(&result, &resolved);
        if kind.acts() && self.config.lane == Lane::Background {
            self.fresh = None;
            let _ = self.list(None);
        }
        Ok(result)
    }

    fn forward(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        self.desktop
            .call(tool, args)
            .map_err(|why| failure(&format!("agent-desktop: {why}")))
    }

    /// The Here lane takes its window once.
    fn ensure_here(&mut self) {
        if self.here_ready {
            return;
        }
        self.here_ready = true;
        let Some(here) = self.config.here_window.clone() else {
            return;
        };
        let window = match &here.window_id {
            Some(id) => Some((id.clone(), here.app.clone())),
            None => self.list(None).ok().and_then(|windows| {
                let of_app: Vec<&Window> = windows
                    .iter()
                    .filter(|window| window.visible && same_app(&window.app, &here.app))
                    .collect();
                of_app
                    .iter()
                    .find(|window| window.focused)
                    .or(if of_app.len() == 1 {
                        of_app.first()
                    } else {
                        None
                    })
                    .map(|window| (window.id.clone(), window.app.clone()))
            }),
        };
        if let Some((id, app)) = window {
            let now = crate::desktop::events::now_ms();
            let taken = self
                .store
                .acquire(&self.config.task, self.pid, &id, &app, now);
            if let Acquire::Taken(other) = taken {
                tracing::warn!(window = id, other, "the Here window is another task's");
            }
        }
    }

    /// The task's windows now.
    fn held(&self) -> Vec<LeaseFile> {
        let mut held = self.store.held_by(&self.config.task);
        if self.config.lane == Lane::Here {
            let here = self
                .config
                .here_window
                .as_ref()
                .and_then(|window| window.window_id.clone());
            if let Some(here) = here {
                held.retain(|lease| lease.window_id == here);
            }
        }
        held
    }

    /// `list-windows`, once per call (a launch looks again); in the
    /// Background lane, new windows of apps the task works in become its.
    fn list(&mut self, launched: Option<&Launched>) -> Result<Vec<Window>, Value> {
        if launched.is_none()
            && let Some(fresh) = &self.fresh
        {
            return Ok(fresh.clone());
        }
        let result = self
            .desktop
            .call("desktop_list_windows", &Map::new())
            .map_err(|why| failure(&format!("agent-desktop: {why}")))?;
        let windows = child::data(&result)
            .map(|data| child::windows(&data))
            .ok_or_else(|| {
                failure("Hex couldn't list the windows to check this call; try again.")
            })?;
        if self.config.lane == Lane::Background {
            self.adopt(&windows, launched);
        }
        self.fresh = Some(windows.clone());
        Ok(windows)
    }

    fn adopt(&mut self, windows: &[Window], launched: Option<&Launched>) {
        let now = Instant::now();
        self.watched
            .retain(|_, since| now.duration_since(*since) < WATCH);
        let on_screen: HashSet<String> = windows
            .iter()
            .filter(|window| window.visible)
            .map(|window| window.id.clone())
            .collect();
        let Some(known) = self.known.replace(on_screen) else {
            return;
        };
        for window in windows.iter().filter(|window| window.visible) {
            if known.contains(&window.id) || self.blocked(&window.app) {
                continue;
            }
            let from_launch = launched.is_some_and(|launched| {
                launched.pid.is_some() && launched.pid == window.pid
                    || same_app(&launched.app, &window.app)
            });
            let watched =
                !window.focused && self.watched.contains_key(&policy::app_key(&window.app));
            if !from_launch && !watched {
                continue;
            }
            let acquired = self.store.acquire(
                &self.config.task,
                self.pid,
                &window.id,
                &window.app,
                crate::desktop::events::now_ms(),
            );
            if acquired == Acquire::Acquired {
                self.notes.push(format!(
                    "the new window {} is yours now.",
                    describe(&window.id, &window.app, &window.title)
                ));
            }
        }
    }

    fn watch(&mut self, app: &str) {
        self.watched.insert(policy::app_key(app), Instant::now());
    }

    /// Waits while the task is paused; an action that waited, or the first
    /// after a resume, is not sent: the window may have changed.
    fn gate(&mut self, tool: &str) -> Result<(), Value> {
        if self.store.paused(&self.config.task) {
            while self.store.paused(&self.config.task) {
                std::thread::sleep(self.poll);
            }
            self.resumed = self.store.resumed(&self.config.task);
            self.note_pending = false;
            return Err(resumed_refusal(tool));
        }
        if self.note_pending {
            self.note_pending = false;
            return Err(resumed_refusal(tool));
        }
        Ok(())
    }

    fn notice_resume(&mut self) {
        let resumed = self.store.resumed(&self.config.task);
        if resumed != self.resumed {
            self.resumed = resumed;
            self.note_pending = true;
        }
    }

    fn launch(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let activate = args.get("activate").and_then(Value::as_bool) == Some(true);
        if activate && !self.config.physical_input {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "activate brings the app in front of the person. Launch without it, then \
                 desktop_wait with event window-opened; if no window opens, launch again with a \
                 file in args.",
            ));
        }
        let app = text(args, "app").unwrap_or_default();
        let _ = self.list(None);
        let hands = (self.config.physical_input && activate)
            .then(|| self.store.hands_on(&self.config.task, self.pid));
        let result = self.forward(tool, args);
        drop(hands);
        let result = result?;
        let data = child::data(&result);
        let launched = Launched {
            app: data
                .as_ref()
                .and_then(|data| data.get("app")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| app.clone()),
            pid: data.as_ref().and_then(|data| data.get("pid")?.as_i64()),
        };
        self.watch(&app);
        self.watch(&launched.app);
        let _ = self.list(Some(&launched));
        Ok(result)
    }

    fn claim(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        if self.config.lane == Lane::Here {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This task acts only in the window that was in front when it was asked.",
            ));
        }
        let id = text(args, "window_id").ok_or_else(|| {
            refusal(
                tool,
                "INVALID_ARGS",
                "Name the window to claim with window_id (from desktop_list_windows).",
            )
        })?;
        let windows = self.list(None)?;
        let window = windows
            .iter()
            .find(|window| window.id == id && window.visible)
            .cloned()
            .ok_or_else(|| {
                refusal(
                    tool,
                    "WINDOW_NOT_FOUND",
                    &format!("There's no window {id} on screen now."),
                )
            })?;
        if self.blocked(&window.app) {
            return Err(self.blocked_refusal(tool, &window.app));
        }
        if window.focused {
            return Err(refusal(
                tool,
                "NOT_YOUR_WINDOW",
                "That's the window in front of the person: it's theirs. Claim another, or open \
                 your own with desktop_launch.",
            ));
        }
        let now = crate::desktop::events::now_ms();
        match self
            .store
            .acquire(&self.config.task, self.pid, &window.id, &window.app, now)
        {
            Acquire::Taken(_) => Err(refusal(
                tool,
                "NOT_YOUR_WINDOW",
                "That window belongs to another task. Open your own window with desktop_launch, \
                 or claim a free one.",
            )),
            Acquire::Acquired | Acquire::Held => {
                self.watch(&window.app);
                let envelope = json!({
                    "version": "hex-guard",
                    "ok": true,
                    "command": "claim-window",
                    "data": {"window": {"id": window.id, "app_name": window.app, "title": window.title}},
                });
                Ok(json!({
                    "content": [{"type": "text", "text": envelope.to_string()}],
                    "structuredContent": envelope,
                    "isError": false,
                }))
            }
        }
    }

    /// Remembers which window each snapshot in `result` was taken of.
    fn record(&mut self, result: &Value, resolved: &[Resolved]) {
        let Some(data) = child::data(result) else {
            return;
        };
        let mut snapshots = Vec::new();
        policy::snapshot_ids(&data, &mut snapshots);
        if snapshots.is_empty() {
            return;
        }
        let single = match resolved {
            [only] => Some(only),
            _ => None,
        };
        let window = data
            .get("window")
            .and_then(|window| window.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| single.and_then(|only| only.id.clone()));
        let app = data
            .get("app")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| single.map(|only| only.app.clone()))
            .unwrap_or_default();
        for snapshot in snapshots {
            let origin = Origin {
                window: window.clone(),
                app: app.clone(),
            };
            self.snapshots.entry(snapshot).or_insert(origin);
        }
    }

    fn windows_note(&mut self) {
        let held = self.held();
        let mine: Vec<String> = held
            .iter()
            .map(|lease| format!("{} ({})", lease.window_id, lease.app))
            .collect();
        self.notes.push(match self.config.lane {
            Lane::Here => format!(
                "this task works only in {}.",
                if mine.is_empty() {
                    "the window that was in front".into()
                } else {
                    mine.join(", ")
                }
            ),
            Lane::Background if mine.is_empty() => {
                "no window is yours yet: open one with desktop_launch or claim a free one with \
                 desktop_claim_window. The focused window is the person's."
                    .into()
            }
            Lane::Background => format!(
                "your windows: {}. The focused window is the person's.",
                mine.join(", ")
            ),
        });
    }
}

fn name_window(args: &mut Map<String, Value>, id: &str) {
    args.insert("window_id".into(), Value::String(id.to_owned()));
    args.remove("app");
}

fn text(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn same_app(a: &str, b: &str) -> bool {
    policy::app_key(a) == policy::app_key(b)
}

fn describe(id: &str, app: &str, title: &str) -> String {
    if title.is_empty() || title == app {
        format!("{id} ({app})")
    } else {
        format!("{id} ({app}, “{title}”)")
    }
}

fn describe_task_window(window: &TaskWindow) -> String {
    match &window.window_id {
        Some(id) => format!("{id} ({})", window.app),
        None => window.app.clone(),
    }
}

/// `(app key, bundle id key)` for each app in `list-apps`' data.
fn bundle_ids(data: &Value) -> Vec<(String, String)> {
    data.get("apps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|app| {
            Some((
                policy::app_key(app.get("name")?.as_str()?),
                policy::app_key(app.get("bundle_id")?.as_str()?),
            ))
        })
        .collect()
}

/// agent-desktop's command name for a tool (`desktop_scroll_to` →
/// `scroll-to`).
fn command(tool: &str) -> String {
    tool.strip_prefix("desktop_")
        .unwrap_or(tool)
        .replace('_', "-")
}

/// A refused call: an agent-desktop error envelope, never delivered.
pub fn refusal(tool: &str, code: &str, message: &str) -> Value {
    let envelope = json!({
        "version": "hex-guard",
        "ok": false,
        "command": command(tool),
        "error": {
            "code": code,
            "message": message,
            "details": {"kind": "hex_guard", "retryable": false},
            "disposition": {"delivery": "not_delivered", "retry": "safe"},
        },
    });
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": true,
    })
}

fn resumed_refusal(tool: &str) -> Value {
    let mut result = refusal(tool, "TAKEN_OVER", RESUMED);
    result["structuredContent"]["error"]["details"]["retryable"] = Value::Bool(true);
    let text = result["structuredContent"].to_string();
    result["content"][0]["text"] = Value::String(text);
    result
}

fn failure(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

fn claim_tool() -> Value {
    json!({
        "name": policy::CLAIM_TOOL,
        "description": "Hex: take a window that's on screen and free (no other task holds it, \
            and it isn't the window in front of the person) so this task may read and operate \
            it. Windows this task opens with desktop_launch are its own already.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id from desktop_list_windows (w-1234)"}
            },
            "required": ["window_id"],
        },
    })
}
