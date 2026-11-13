//! One task's guard: every tool call is checked against the task's lane and
//! leases, then forwarded to agent-desktop or refused with a tool error the
//! model can act on. Synchronous; `server` runs it off the MCP runtime.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

mod actions;
mod apps;
mod borrow;
mod consent;
mod desk;
mod displays;
mod front;
mod lookup;
mod recover;
mod shared;
mod vision;

pub(super) use desk::DESK_NOTE;
pub use recover::{Inert, Recoverer, System};
pub use shared::Seed;
use shared::{Learned, Shared};

use super::child::{self, Desktop, Window};
use super::policy::{self, Kind};
use super::screen::Screen;
use super::state::{Acquire, LeaseFile, Store};
use crate::assistant::types::{Lane, TaskWindow};

/// How long an action waits for another task's to give the real pointer
/// and keyboard back.
const HANDS_WAIT: Duration = Duration::from_secs(20);

/// What the guard knows about its task.
#[derive(Clone, Debug)]
pub struct Config {
    pub task: String,
    pub lane: Lane,
    pub here_window: Option<TaskWindow>,
    pub physical_input: bool,
    pub blocked_apps: Vec<String>,
    /// `assistant.agent_display`: where new Background windows go.
    pub agent_display: Option<String>,
    /// `assistant.autonomy`: how freely it takes consequential actions.
    pub autonomy: crate::assistant::autonomy::Autonomy,
    /// A Background task may claim the window in front of the person.
    pub front_window: bool,
    /// The task may open apps and use them in windows of its own, parked
    /// out of the person's way (implied by `front_window`).
    pub open_apps: bool,
    /// Reads only (a job's check): every action is refused.
    pub observe_only: bool,
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

/// How long a look at the windows on screen (and so at the one in front)
/// stands for the next.
const FRESH: Duration = Duration::from_millis(150);

/// Why the guard looks at the windows on screen.
#[derive(Clone, Copy)]
enum Look<'a> {
    /// To check a call: a look up to [`FRESH`] old will do.
    Check,
    /// Right after the task's own action: new windows may be its.
    AfterAction,
    /// Right after a launch: the app's new windows are its.
    Launched(&'a Launched),
}

pub const RESUMED: &str = "The person used this window while this task was paused; take a fresh \
snapshot before acting. (The action you asked for was not sent.)";

pub struct Guard<D: Desktop> {
    pub(super) config: Config,
    store: Store,
    pid: u32,
    /// The driver, shared with this guard's readers (`shared`).
    desktop: Shared<D>,
    screen: Shared<Box<dyn Screen>>,
    /// Snapshots' windows, refs' labels, element spots, and window
    /// pictures, shared with this guard's readers.
    learned: Learned,
    /// Window ids on screen at the last look (Background: new ones may be
    /// adopted).
    known: Option<HashSet<String>>,
    watched: HashMap<String, Instant>,
    /// Windows this call made the task's, to move to the agents' display.
    arrived: Vec<Window>,
    /// Windows moved to a display, by window id: the display's id.
    placed: HashMap<String, String>,
    /// The last look at the windows on screen, and when.
    looked: Option<(Instant, Vec<Window>)>,
    notes: Vec<String>,
    resumed: Option<String>,
    note_pending: bool,
    here_ready: bool,
    bundle_ids: Option<Vec<(String, String)>>,
    /// Time this call spent in agent-desktop.
    forwarding: Duration,
    /// How long a look at the windows stands for the next check.
    pub fresh_for: Duration,
    /// How long `hex_open_app` waits for a new window.
    pub window_wait: Duration,
    /// How often a paused action looks again.
    pub poll: Duration,
    /// Asks a running app to show a window again (`open -g`); a stand-in in
    /// tests, which never open the person's apps.
    pub reopen: Box<dyn Fn(&super::screen::App) + Send>,
    /// Brings an app's window from another desktop, the Dock, or hiding
    /// (`recover`); a stand-in in tests.
    pub recover: Box<dyn Recoverer>,
    /// Runs an app action (`actions::app_action`); a stand-in in tests.
    pub run_action: Box<RunAction>,
    /// Reads what an installed app offers (`actions::capabilities`); a
    /// stand-in in tests, which never read the person's apps.
    pub discover: Box<dyn Fn(&super::screen::App) -> Value + Send>,
    /// Hex's own process, which holds the desk (`desk`); `None`: no desk.
    pub desk: Option<u32>,
    /// Failed actions in a row per window (background first: `borrow`).
    failures: HashMap<String, u32>,
    /// Windows a focus was asked for once (and refused).
    focus_asked: HashSet<String>,
    /// The window brought forward briefly, if one is.
    borrowed: Option<borrow::Borrowed>,
    /// A reader beside the task's guard: read-only calls only, and none of
    /// the guard's own upkeep (`shared`).
    reader: bool,
}

/// Runs one app action and says what it printed, or why not.
pub type RunAction =
    dyn FnMut(&crate::assistant::actions::app_action::Command) -> Result<String, String> + Send;

impl<D: Desktop> Guard<D> {
    pub fn new(
        config: Config,
        store: Store,
        pid: u32,
        desktop: D,
        screen: Box<dyn Screen>,
    ) -> Self {
        Self::assemble(
            config,
            store,
            pid,
            Shared::new(desktop),
            Shared::new(screen),
            Learned::default(),
        )
    }

    fn assemble(
        config: Config,
        store: Store,
        pid: u32,
        desktop: Shared<D>,
        screen: Shared<Box<dyn Screen>>,
        learned: Learned,
    ) -> Self {
        let resumed = store.resumed(&config.task);
        Self {
            config,
            store,
            pid,
            desktop,
            screen,
            learned,
            known: None,
            watched: HashMap::new(),
            arrived: Vec::new(),
            placed: HashMap::new(),
            looked: None,
            notes: Vec::new(),
            resumed,
            note_pending: false,
            here_ready: false,
            bundle_ids: None,
            forwarding: Duration::ZERO,
            fresh_for: FRESH,
            window_wait: apps::WINDOW_WAIT,
            poll: Duration::from_millis(250),
            // Tests never open the person's apps.
            reopen: Box::new(|app| {
                if !cfg!(test) {
                    apps::reopen(app);
                }
            }),
            recover: if cfg!(test) {
                Box::new(Inert)
            } else {
                Box::new(System)
            },
            run_action: Box::new(|command| {
                if cfg!(test) {
                    Ok(String::new())
                } else {
                    crate::assistant::actions::app_action::run(command)
                }
            }),
            discover: Box::new(|app| {
                if cfg!(test) {
                    serde_json::json!({"app": app.name, "bundle_id": app.bundle_id})
                } else {
                    crate::assistant::actions::capabilities::describe(
                        &app.name,
                        app.bundle_id.as_deref(),
                        &app.path,
                    )
                }
            }),
            desk: None,
            failures: HashMap::new(),
            focus_asked: HashSet::new(),
            borrowed: None,
            reader: false,
        }
    }

    /// The driver, while the returned guard lives.
    pub fn desktop(&mut self) -> std::sync::MutexGuard<'_, D> {
        self.desktop.lock()
    }

    /// Whether a Here task opens apps and uses them in windows of its own
    /// (full control, or allowed to); Background tasks always may.
    fn opens_apps(&self) -> bool {
        self.config.open_apps || self.config.front_window
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
                Kind::Refused(_)
                | Kind::Claim
                | Kind::InstalledApps
                | Kind::OpenApp
                | Kind::Displays => false,
                Kind::Physical => physical,
                Kind::Clipboard => !background || physical,
                Kind::Launch => background,
                // Background first: a Background task brings a window
                // forward (briefly) only with full control.
                Kind::FocusWindow => !background || self.config.front_window,
                _ => true,
            }
        });
        // Full control reaches every app from the Here lane too: claim a
        // window, open or switch to an app.
        let reach = background || self.opens_apps();
        if reach {
            tools.push(claim_tool());
        }
        tools.extend(apps::tools(reach));
        tools.push(displays::tool());
        if reach {
            tools.push(desk::tool());
            tools.push(actions::app_action_tool());
        }
        tools.push(actions::capabilities_tool());
        tools.push(actions::read_window_tool());
        tools.push(vision::shot_tool());
        tools.push(vision::input_tool());
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
            Lane::Here if self.config.front_window => format!(
                "Hex guards these tools. The person gave this task full control: it may read and \
                 act in any window no other task holds, the one in front ({}) included; it claims \
                 one with desktop_claim_window, and opens or switches to an app with hex_open_app \
                 (never a second instance of a running app). Blocked apps are refused. {input}",
                self.config
                    .here_window
                    .as_ref()
                    .map_or("unknown".into(), describe_task_window)
            ),
            Lane::Here if self.config.open_apps => format!(
                "Hex guards these tools. This task stays out of the person's windows: it may act \
                 in the window that was in front ({}) only, and in apps it opens or switches to \
                 with hex_open_app (their windows become its own, out of the person's way; never \
                 a second instance), or free windows it claims that aren't in front. Blocked apps \
                 are refused. {input}",
                self.config
                    .here_window
                    .as_ref()
                    .map_or("unknown".into(), describe_task_window)
            ),
            Lane::Here => format!(
                "Hex guards these tools. This task was asked about the window in front ({}); it \
                 may read and act only in that window, and in new windows of that app its own \
                 actions open (a dialog, a panel). Other windows, launching apps, and blocked \
                 apps are refused. {input}",
                self.config
                    .here_window
                    .as_ref()
                    .map_or("unknown".into(), describe_task_window)
            ),
            Lane::Background => format!(
                "Hex guards these tools. This task runs in the background beside the person: it \
                 may read and act only in windows it holds, which are windows it opened with \
                 desktop_launch (and new windows its own actions open) and free windows it took \
                 with desktop_claim_window. {front} \
                 desktop_list_windows and desktop_list_apps see everything. Pass window_id when \
                 an app has several windows. {input}",
                front = if self.config.front_window {
                    "The person allowed it the window in front too: claim it by its window_id."
                } else {
                    "Never the window in front of the person."
                }
            ),
        }
    }

    /// Checks and forwards one call; the MCP `CallToolResult` to return.
    pub fn call(&mut self, tool: &str, args: Map<String, Value>) -> Value {
        let started = Instant::now();
        self.notes.clear();
        self.forwarding = Duration::ZERO;
        self.notice_resume();
        self.refresh_here_window();
        let mut args = args;
        // A reader leaves the guard's upkeep to the guard.
        if self.config.lane == Lane::Background && !self.reader {
            self.return_orphans();
        }
        if !self.reader {
            self.borrow_expired();
        }
        let outcome = self.run(tool, &mut args);
        self.place_arrivals();
        tracing::debug!(
            tool,
            guard_ms = started
                .elapsed()
                .saturating_sub(self.forwarding)
                .as_secs_f64()
                * 1000.0,
            forward_ms = self.forwarding.as_secs_f64() * 1000.0,
            refused = outcome.is_err(),
            "guarded call"
        );
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
        policy::lean_snapshot(tool, args);
        Self::want_bounds(tool, args);
        let kind = policy::kind(tool, args);
        if self.reader && !kind.reads_alone() {
            return Err(failure("Hex: this call isn't one a reader runs."));
        }
        if let Some(app) = text(args, "app")
            && self.blocked(&app)
        {
            return Err(self.blocked_refusal(tool, &app));
        }
        if tool.starts_with("desktop_") && kind.needs_desktop() {
            self.running_name(args);
        }
        // A locked screen stops every desktop step; say so instead of
        // failing silently (a sleeping display is woken on the way).
        if kind.needs_desktop()
            && crate::desktop::screen_lock::desktop_state()
                == crate::desktop::screen_lock::DesktopState::Locked
        {
            return Err(refusal(
                tool,
                "SCREEN_LOCKED",
                "The Mac's screen is locked, so no desktop step can run until the person unlocks it. Do every part that needs no desktop now (browser tools, files, shell); if desktop steps are still left, ask the person once: start your reply with `HEX_QUESTION:` and say the screen is locked and what is waiting for it.",
            ));
        }
        if self.config.lane == Lane::Here {
            self.ensure_here();
        }
        if self.config.observe_only
            && !matches!(
                kind,
                Kind::Free
                    | Kind::Observe
                    | Kind::InstalledApps
                    | Kind::AppCapabilities
                    | Kind::Displays
                    | Kind::ReadWindow
                    | Kind::WindowShot
            )
        {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "This is a check: look and read only, change nothing.",
            ));
        }
        match kind {
            Kind::Refused(why) => return Err(refusal(tool, "POLICY_DENIED", why)),
            Kind::Claim => return self.claim(tool, args),
            Kind::InstalledApps => return Ok(self.installed_apps(args)),
            Kind::Displays => return Ok(self.displays_tool()),
            Kind::Release => {
                if !self.reaches_desk() {
                    return Err(refusal(
                        tool,
                        "POLICY_DENIED",
                        "This task acts only in the window that was in front when it was asked.",
                    ));
                }
                return self.release_tool(args);
            }
            Kind::ReadWindow => return self.read_window(tool, args),
            Kind::WindowShot => return self.window_shot(tool, args),
            Kind::WindowInput => return self.window_input(tool, args),
            Kind::AppCapabilities => return self.app_capabilities(tool, args),
            Kind::Clipboard
                if self.config.lane == Lane::Background && !self.config.physical_input =>
            {
                return Err(refusal(
                    tool,
                    "POLICY_DENIED",
                    "The clipboard is shared with the person; a Background task puts text in its \
                     window with desktop_type or desktop_set_value instead.",
                ));
            }
            Kind::Clipboard => return self.forward(tool, args),
            Kind::Free => {
                let result = self.forward(tool, args)?;
                let background = self.config.lane == Lane::Background;
                if background && tool == "desktop_wait" && args.contains_key("event") {
                    let _ = self.list(Look::AfterAction);
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
        if self.config.lane == Lane::Here && matches!(kind, Kind::Launch) && !self.opens_apps() {
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
        if kind == Kind::OpenApp {
            return self.open_app(args);
        }
        if kind == Kind::AppAction {
            if self.config.lane == Lane::Here && !self.opens_apps() {
                return Err(refusal(
                    tool,
                    "POLICY_DENIED",
                    "This task was asked about the window in front and acts only there.",
                ));
            }
            return self.app_action(tool, args);
        }
        if kind == Kind::FocusWindow
            && self.config.lane == Lane::Background
            && !self.config.front_window
        {
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
        self.borrow_desk(tool, kind, &resolved)?;
        self.secure_read(tool, args)?;
        if kind.acts() {
            self.consent(tool, args, &resolved)?;
            if self.config.lane == Lane::Background || self.opens_apps() {
                for window in &resolved {
                    self.watch(&window.app);
                }
            }
            // What was on screen before, so what its action opens is new.
            let _ = self.list(Look::Check);
        }
        if kind.acts() {
            self.borrow_step(&resolved);
        }
        if kind == Kind::FocusWindow {
            let result = self.focus(tool, args, &resolved)?;
            let _ = self.list(Look::AfterAction);
            return Ok(result);
        }
        let hands = if kind.acts() && self.config.physical_input {
            Some(self.take_hands(tool)?)
        } else {
            None
        };
        let result = self.forward(tool, args);
        drop(hands);
        let mut result = result?;
        self.learn_and_redact(&mut result);
        self.learn_frames(&result);
        self.record(&result, &resolved);
        if kind.acts() {
            result = self.ax_fallback(tool, args, &resolved, result);
            self.count_failure(&resolved, &result);
            if kind == Kind::CloseApp
                && child::data(&result).is_some()
                && let Some(window) = resolved.first()
            {
                let app = window.app.clone();
                self.closed_app(&app);
            }
            let _ = self.list(Look::AfterAction);
            self.borrow_done();
        }
        Ok(result)
    }

    /// The real pointer and keyboard for this action, one task at a time
    /// (a second task's action would move the same pointer): waits a while
    /// for another task's action to end, else refuses as busy.
    fn take_hands(&self, tool: &str) -> Result<super::state::HandsOn, Value> {
        self.store
            .take_hands(&self.config.task, self.pid, HANDS_WAIT)
            .map_err(|_| {
                refusal(
                    tool,
                    "BUSY",
                    "Another task is using the real mouse and keyboard right now. Try again in a \
                     moment, or do it with a headless action.",
                )
            })
    }

    fn forward(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let started = Instant::now();
        let result = self
            .desktop
            .call(tool, args)
            .map_err(|why| failure(&format!("agent-desktop: {why}")));
        self.forwarding += started.elapsed();
        result
    }

    /// A Here task's window as Hex last recorded it for this request
    /// (`guard::mcp_server`): a guard started ahead in another window takes
    /// the request's own window before its first call.
    fn refresh_here_window(&mut self) {
        if self.config.lane != Lane::Here {
            return;
        }
        let Some(window) = self.store.here_window(&self.config.task) else {
            return;
        };
        let window = TaskWindow {
            app: window.app,
            window_id: window.window_id,
        };
        if self.config.here_window.as_ref() != Some(&window) {
            self.config.here_window = Some(window);
            self.here_ready = false;
        }
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
            None => self.list(Look::Check).ok().and_then(|windows| {
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

    /// The task's windows now (a Here task's are all in its app).
    fn held(&self) -> Vec<LeaseFile> {
        let mut held = self.store.held_by(&self.config.task);
        // The desk's windows are every app-opening task's to act in.
        for lease in self.desk_leases() {
            if !held.iter().any(|mine| mine.window_id == lease.window_id) {
                held.push(lease);
            }
        }
        if self.opens_apps() {
            return held;
        }
        if let (Lane::Here, Some(here)) = (self.config.lane, &self.config.here_window) {
            held.retain(|lease| {
                here.window_id.as_ref() == Some(&lease.window_id) || same_app(&lease.app, &here.app)
            });
        }
        held
    }

    /// The windows on screen, read in this process (`screen`); a look up
    /// to [`FRESH`] old stands for a check. New windows the task's own
    /// actions open become its (`adopt`).
    fn list(&mut self, look: Look<'_>) -> Result<Vec<Window>, Value> {
        if let (Look::Check, Some((at, windows))) = (look, &self.looked)
            && at.elapsed() < self.fresh_for
        {
            return Ok(windows.clone());
        }
        let mut windows = self.screen.windows();
        if windows.is_empty() {
            return Err(failure(
                "Hex couldn't see the windows on screen to check this call; try again.",
            ));
        }
        // The task's own windows that aren't on screen (on another Space,
        // minimized) are still its to use; they can't be in front.
        for lease in self.held() {
            if !windows.iter().any(|window| window.id == lease.window_id) {
                windows.push(Window {
                    id: lease.window_id,
                    app: lease.app,
                    title: String::new(),
                    pid: None,
                    focused: false,
                    visible: true,
                    minimized: false,
                    bounds: None,
                });
            }
        }
        self.adopt(&windows, look);
        self.looked = Some((Instant::now(), windows.clone()));
        Ok(windows)
    }

    /// Takes new windows that are the task's own doing: a Background task's
    /// in an app it launched or works in (never one in front, which the
    /// person opened), a Here task's in its own app right after its action.
    fn adopt(&mut self, windows: &[Window], look: Look<'_>) {
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
        let here = self.config.here_window.clone().map(|here| {
            let pid = here.window_id.as_ref().and_then(|id| {
                windows
                    .iter()
                    .find(|window| &window.id == id)
                    .and_then(|window| window.pid)
            });
            (here.app, pid)
        });
        for window in windows.iter().filter(|window| window.visible) {
            if known.contains(&window.id) || self.blocked(&window.app) {
                continue;
            }
            let full = self.opens_apps();
            let ours = match (self.config.lane, look) {
                (Lane::Here, Look::AfterAction) => {
                    here.as_ref().is_some_and(|(app, pid)| match pid {
                        Some(pid) => window.pid == Some(*pid),
                        None => same_app(app, &window.app),
                    }) || (full
                        && !window.focused
                        && self.watched.contains_key(&policy::app_key(&window.app)))
                }
                // With full control the Here lane opens apps too.
                (Lane::Here, Look::Launched(launched)) if full => launched_by(launched, window),
                (Lane::Here, _) => false,
                (Lane::Background, Look::Launched(launched)) => launched_by(launched, window),
                (Lane::Background, _) => {
                    !window.focused && self.watched.contains_key(&policy::app_key(&window.app))
                }
            };
            if !ours {
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
                // A Background task's new windows, and the apps a Here task
                // opens, go out of the person's way.
                let launched = matches!(look, Look::Launched(_));
                if self.config.lane == Lane::Background || launched {
                    self.arrived.push(window.clone());
                }
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
        let _ = self.list(Look::Check);
        let hands = if self.config.physical_input && activate {
            Some(self.take_hands(tool)?)
        } else {
            None
        };
        // The person's app keeps the front (`front`): noted before.
        let before = (self.config.lane == Lane::Background && !activate)
            .then(front::frontmost)
            .flatten();
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
        let _ = self.list(Look::Launched(&launched));
        front::keep_front(before, launched.pid);
        // Onto the agents' display now, not after the call.
        self.place_arrivals();
        Ok(result)
    }

    fn claim(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        if self.config.lane == Lane::Here && !self.opens_apps() {
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
        let windows = self.list(Look::Check)?;
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
        if window.focused && !self.config.front_window {
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
            Acquire::Taken(task) if !self.usable(&window.id, &Acquire::Taken(task.clone())) => {
                Err(self.desk_refusal(tool, &window.id).unwrap_or_else(|| {
                    refusal(
                        tool,
                        "NOT_YOUR_WINDOW",
                        "That window belongs to another task. Open your own window with \
                         desktop_launch, or claim a free one.",
                    )
                }))
            }
            Acquire::Taken(_) => {
                self.watch(&window.app);
                let envelope = json!({
                    "version": "hex-guard",
                    "ok": true,
                    "command": "claim-window",
                    "data": {"window": {"id": window.id, "app_name": window.app, "title": window.title}, "desk": true},
                });
                Ok(json!({
                    "content": [{"type": "text", "text": envelope.to_string()}],
                    "structuredContent": envelope,
                    "isError": false,
                }))
            }
            acquired @ (Acquire::Acquired | Acquire::Held) => {
                // A window the task takes goes out of the person's way and
                // shows live on its card, unless it is the one they're in.
                if acquired == Acquire::Acquired && !window.focused {
                    self.arrived.push(window.clone());
                }
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
            self.learned
                .snapshots
                .lock()
                .entry(snapshot)
                .or_insert(origin);
        }
    }

    fn windows_note(&mut self) {
        let held = self.held();
        let mine: Vec<String> = held
            .iter()
            .map(|lease| format!("{} ({})", lease.window_id, lease.app))
            .collect();
        self.notes.push(match self.config.lane {
            Lane::Here if self.config.front_window => format!(
                "with full control any window no other task holds is in reach (claim it by its \
                 id); yours now: {}.",
                if mine.is_empty() {
                    "the window that was in front".into()
                } else {
                    mine.join(", ")
                }
            ),
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

impl<D: Desktop> Guard<D> {
    /// Names the app in `args["app"]` exactly as it runs: a name the agent
    /// typed ("WhatsApp") becomes the running app's own ("\u{200e}WhatsApp",
    /// with its invisible mark), so agent-desktop finds it.
    fn running_name(&mut self, args: &mut Map<String, Value>) {
        let Some(app) = text(args, "app") else {
            return;
        };
        let windows = self.list(Look::Check).unwrap_or_default();
        let real = windows
            .iter()
            .find(|window| window.visible && same_app(&window.app, &app))
            .or_else(|| windows.iter().find(|window| same_app(&window.app, &app)))
            .map(|window| window.app.clone());
        if let Some(real) = real
            && real != app
        {
            args.insert("app".into(), Value::String(real));
        }
    }
}

impl<D: Desktop> Drop for Guard<D> {
    /// A window brought forward briefly goes back as the task ends.
    fn drop(&mut self) {
        self.restore_borrowed();
    }
}

/// Whether `window` is one of the app `launched` just started (or reopened).
fn launched_by(launched: &Launched, window: &Window) -> bool {
    launched.pid.is_some() && launched.pid == window.pid || same_app(&launched.app, &window.app)
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
