//! The guard between every task and the desktop: Hex's own MCP server
//! (`hex assistant mcp`), which each task's agent gets instead of
//! agent-desktop directly. It forwards agent-desktop's tools and enforces
//! what the lanes promise, so a prompt is never the only safeguard:
//!
//! - **Window leases.** A task acts only in windows it holds. The Here lane
//!   holds the window that was in front when asked (and new windows its own
//!   actions open in that app, such as a dialog); a Background task holds
//!   windows it opened (`launch`, new windows after its actions) or claimed
//!   while no other task holds them. Nobody acts in another task's window,
//!   and Background tasks never act in the window in front of the person.
//!   Leases live as files under `<support>/assistant/leases/`, created
//!   exclusively, so separate processes agree; a dead holder's lease lapses
//!   (its pid gone, or reused by a process that started at another time).
//! - **Input policy.** Real pointer and keys (`--headed`) only for a Here task
//!   with `here_physical_input`, or a Background task with `background_physical_input`; while one
//!   runs, `hands_off()` names it for the HUD. The clipboard, shared with the
//!   person, only for Here tasks or with real input.
//! - **Blocked apps.** No task reads or operates `blocked_apps`.
//! - **Take over.** While a task is paused its actions wait; when handed back
//!   its next result tells it to look again before acting.
//!
//! Windows on screen, and the one in front, are read in the guard's process
//! from CoreGraphics (`screen`, under a millisecond), so agent-desktop runs
//! only the task's own calls. Hex adds `desktop_claim_window`,
//! `hex_installed_apps`, and `hex_open_app` to agent-desktop's tools.
//!
//! `hex assistant mcp --spec <json or file>` serves it (`server`); the spec
//! is [`WireSpec`] as JSON. `mcp_server` gives a task's agent that command.
//!
//! OWNER: guard.

mod child;
mod cua;
mod desk;
mod parking;
mod policy;
mod proxy;
mod screen;
mod server;
mod state;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::assistant::backends::McpServer;
use crate::assistant::types::{ConversationId, Lane, TaskWindow};

pub use desk::{
    DESK, desk, desk_browsers, desk_keep_browser, desk_release, desk_release_app,
    desk_release_browser,
};
pub use parking::{How as ParkedHow, Mover as ParkingMover, ParkedWindow, Rect, back_to};
pub use policy::{CLAIM_TOOL, OPEN_APP_TOOL, app_key};

/// What the guard needs to know about one task; passed to its MCP server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSpec {
    pub task: ConversationId,
    pub lane: Lane,
    /// agent-desktop's agent id (its own cursor) and multi-agent session.
    pub agent_id: String,
    pub session: Option<String>,
    /// The Here lane's window.
    pub here_window: Option<TaskWindow>,
    /// Whether real input is allowed for this task (already resolved from
    /// the lane and settings).
    pub physical_input: bool,
    pub blocked_apps: Vec<String>,
    /// Full control: the task may operate any window (the one in front
    /// too) and open any app, in either lane — unless the person asked it
    /// to work only in the background (`assistant::full_control`).
    pub full_control: bool,
    /// The task may open and use apps in windows of its own (parked out of
    /// the person's way), in either lane: full control is allowed in the
    /// settings, even when this request asked to stay out of the person's
    /// windows.
    pub open_apps: bool,
}

/// A [`TaskSpec`] as `hex assistant mcp --spec` reads it (JSON).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireSpec {
    pub task: String,
    /// `"here"` or `"background"`.
    pub lane: Lane,
    pub agent_id: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub here_window: Option<WireWindow>,
    #[serde(default)]
    pub physical_input: bool,
    #[serde(default)]
    pub blocked_apps: Vec<String>,
    /// agent-desktop's executable; `agent-desktop` on `PATH` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_desktop: Option<PathBuf>,
    /// Cua Driver's executable: when given, the desktop driver
    /// (`assistant::driver`), with agent-desktop standing in when it fails
    /// to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cua_driver: Option<PathBuf>,
    /// The guard's folder; `<support>/assistant` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<PathBuf>,
    /// Where the guard sees the windows on screen.
    #[serde(default, skip_serializing_if = "WindowSource::is_default")]
    pub windows: WindowSource,
    /// `assistant.agent_display`: the display new Background windows go to
    /// (an id or name); `None` is the one the person isn't working on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_display: Option<String>,
    /// `assistant.autonomy`: "ask", "smart" (the default), or "yolo".
    #[serde(default)]
    pub autonomy: crate::assistant::autonomy::Autonomy,
    /// `assistant.full_control`: a Background task may claim
    /// the window in front of the person too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub front_window: bool,
    /// A Here task may open and use apps in windows of its own
    /// ([`TaskSpec::open_apps`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open_apps: bool,
    /// Reads only: every action is refused (a job's check, which must
    /// change nothing).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub observe_only: bool,
    /// Hex's own process: the desk's holder (`desk`), so apps a quick turn
    /// opens stay held after the turn ends, for as long as Hex runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desk_pid: Option<u32>,
}

/// Where the guard reads the windows on screen.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowSource {
    /// CoreGraphics, in the guard's process: a few milliseconds.
    #[default]
    CoreGraphics,
    /// agent-desktop's `list-windows` (a few hundred milliseconds), for
    /// tests with a stand-in agent-desktop.
    AgentDesktop,
}

impl WindowSource {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireWindow {
    pub app: String,
    #[serde(default)]
    pub window_id: Option<String>,
}

impl WireSpec {
    pub fn new(spec: &TaskSpec) -> Self {
        Self {
            task: spec.task.clone(),
            lane: spec.lane,
            agent_id: spec.agent_id.clone(),
            session: spec.session.clone(),
            here_window: spec.here_window.as_ref().map(|window| WireWindow {
                app: window.app.clone(),
                window_id: window.window_id.clone(),
            }),
            physical_input: spec.physical_input,
            blocked_apps: spec.blocked_apps.clone(),
            agent_desktop: None,
            cua_driver: None,
            state_dir: None,
            windows: WindowSource::default(),
            agent_display: None,
            autonomy: crate::assistant::autonomy::Autonomy::default(),
            front_window: spec.full_control,
            open_apps: spec.open_apps,
            observe_only: false,
            desk_pid: None,
        }
    }

    /// `text` as JSON, or the JSON file it names.
    pub fn parse(text: &str) -> Result<Self> {
        let json = if text.trim_start().starts_with('{') {
            text.to_owned()
        } else {
            std::fs::read_to_string(text)
                .map_err(|error| eyre!("can't read the spec {text}: {error}"))?
        };
        serde_json::from_str(&json).map_err(|error| eyre!("the spec isn't valid: {error}"))
    }
}

/// The MCP server a task's agent should get in place of agent-desktop:
/// this executable's `assistant mcp --spec <json>` (inside Hex.app, the
/// bundle's binary). `None` when agent-desktop isn't installed.
///
/// The spec names agent-desktop and the guard's folder as this process
/// found them, so the guard agrees with Hex even when an agent starts its
/// MCP servers with an environment of its own.
///
/// A Here task's window goes through the guard's state
/// (`guard/here/<task>.json`), not the arguments: a backend started ahead
/// in one window serves a request asked in another.
pub fn mcp_server(spec: &TaskSpec) -> Option<McpServer> {
    let active = crate::assistant::driver::active()?;
    let agent = crate::assistant::backends::mcp_server(&[]);
    let desktop = agent.clone().unwrap_or_else(|| McpServer {
        command: active.path.clone(),
        args: Vec::new(),
        path_env: crate::assistant::backends::search::path_value(std::slice::from_ref(
            &active.path,
        )),
    });
    let hex = std::env::current_exe().ok()?;
    let store = state::Store::standard()?;
    store.set_here_window(&spec.task, WireSpec::new(spec).here_window.as_ref());
    let mut server = server_for(spec, &hex, &desktop, store.root());
    if active.driver == crate::assistant::driver::Driver::Cua {
        with_cua(&mut server, &active.path, agent.is_some());
    }
    Some(server)
}

/// The desktop tools a task with `spec` is offered (Cua Driver behind the
/// guard): what its system prompt may name (`agent::toolset`).
pub fn offered_tools(spec: &TaskSpec) -> Vec<String> {
    proxy::offered_names(&config(&WireSpec::new(spec)))
}

/// `server` (the guard's) running Cua Driver at `path`, with agent-desktop
/// as its stand-in only when that is installed.
fn with_cua(server: &mut McpServer, path: &Path, agent_desktop: bool) {
    if let Some(spec) = server.args.get_mut(3)
        && let Ok(mut wire) = WireSpec::parse(spec)
    {
        wire.cua_driver = Some(path.to_path_buf());
        if !agent_desktop {
            wire.agent_desktop = None;
        }
        *spec = serde_json::to_string(&wire).unwrap_or_default();
    }
}

/// `server` (the guard's, from [`mcp_server`]) reading only: a job's check
/// sees the windows but every action is refused. Unchanged when it isn't
/// the guard's.
pub fn observe_only(server: &McpServer) -> McpServer {
    let mut server = server.clone();
    if server.args.get(..3) == Some(&["assistant".into(), "mcp".into(), "--spec".into()])
        && let Some(spec) = server.args.get_mut(3)
        && let Ok(mut wire) = WireSpec::parse(spec)
    {
        wire.observe_only = true;
        *spec = serde_json::to_string(&wire).unwrap_or_default();
    }
    server
}

/// The guard's server for a setup check: a read-only Background task, so
/// an agent can call `desktop_status` through whichever driver runs.
pub fn check_server() -> Option<McpServer> {
    let spec = TaskSpec {
        task: "setup-check".into(),
        lane: Lane::Background,
        agent_id: "hex-check".into(),
        session: None,
        here_window: None,
        physical_input: false,
        blocked_apps: Vec::new(),
        full_control: false,
        open_apps: false,
    };
    mcp_server(&spec).map(|server| observe_only(&server))
}

/// Starts Cua Driver as the guard does and lists the running apps and the
/// windows through it: (apps, windows).
pub fn driver_round_trip() -> std::result::Result<(usize, usize), String> {
    use child::Desktop;
    let found = crate::assistant::driver::active().ok_or("no desktop driver is installed")?;
    let backend = found.driver.backend();
    let mut driver = cua::Cua::new(child::AgentDesktop::server(
        found.path,
        backend.mcp_args(None, "", false),
        backend.env(),
    ));
    driver.tools()?;
    let count =
        |result: serde_json::Value, key: Option<&str>| -> std::result::Result<usize, String> {
            let data = child::data(&result).ok_or_else(|| result.to_string())?;
            let list = match key {
                Some(key) => data.get(key).cloned().unwrap_or_default(),
                None => data,
            };
            Ok(list.as_array().map_or(0, Vec::len))
        };
    let apps = count(
        driver.call("desktop_list_apps", &serde_json::Map::new())?,
        Some("apps"),
    )?;
    let windows = count(
        driver.call("desktop_list_windows", &serde_json::Map::new())?,
        None,
    )?;
    Ok((apps, windows))
}

/// Whether process `pid` is running (for the store's conversations).
pub fn process_alive(pid: u32) -> bool {
    state::alive(pid)
}

/// When process `pid` started (µs since the epoch), to tell it from a later
/// process given the same pid.
pub fn process_started_us(pid: u32) -> Option<u64> {
    state::started_us(pid)
}

/// Whether process `pid` still runs and is the one that started at
/// `started` (any process with that pid when `None`).
pub fn process_running(pid: u32, started: Option<u64>) -> bool {
    state::running(pid, started)
}

fn server_for(spec: &TaskSpec, hex: &Path, desktop: &McpServer, state_dir: &Path) -> McpServer {
    let mut wire = WireSpec::new(spec);
    // Read from the state dir ([`mcp_server`]), so the arguments don't
    // depend on the window.
    wire.here_window = None;
    wire.agent_desktop = Some(desktop.command.clone());
    wire.state_dir = Some(state_dir.to_path_buf());
    wire.agent_display = crate::assistant::computer::agent_display_setting();
    // This process (Hex) holds the desk.
    wire.desk_pid = Some(std::process::id());
    if let Some((_, settings)) = crate::app_settings::assistant_runtime() {
        wire.autonomy = settings.autonomy;
    }
    McpServer {
        command: hex.to_path_buf(),
        args: vec![
            "assistant".into(),
            "mcp".into(),
            "--spec".into(),
            serde_json::to_string(&wire).unwrap_or_default(),
        ],
        path_env: desktop.path_env.clone(),
    }
}

/// Runs `hex assistant mcp --spec <spec>` until the agent disconnects.
pub fn serve(spec: &str) -> Result<()> {
    let wire = WireSpec::parse(spec)?;
    let store = match &wire.state_dir {
        Some(dir) => state::Store::new(dir),
        None => state::Store::standard().ok_or_else(|| eyre!("no application support folder"))?,
    };
    store.sweep();
    let program = wire
        .agent_desktop
        .clone()
        .unwrap_or_else(|| PathBuf::from("agent-desktop"));
    let flags = crate::assistant::provider::flags(
        wire.session.as_deref(),
        &wire.agent_id,
        wire.physical_input,
    );
    let screen: Box<dyn screen::Screen> = match wire.windows {
        #[cfg(target_os = "macos")]
        WindowSource::CoreGraphics => {
            screen::CoreGraphics::warm();
            Box::new(screen::CoreGraphics)
        }
        #[cfg(not(target_os = "macos"))]
        WindowSource::CoreGraphics => Box::new(screen::Native(child::AgentDesktop::new(
            program.clone(),
            Vec::new(),
        ))),
        WindowSource::AgentDesktop => Box::new(screen::Listed(child::AgentDesktop::new(
            program.clone(),
            Vec::new(),
        ))),
    };
    let fallback = child::AgentDesktop::new(program, flags);
    let desktop = Driven::new(wire.cua_driver.as_deref(), fallback);
    let pid = std::process::id();
    let mut config = config(&wire);
    if config.here_window.is_none() {
        config.here_window = store.here_window(&wire.task).map(task_window);
    }
    let mut guard = proxy::Guard::new(config, store.clone(), pid, desktop, screen);
    guard.desk = wire.desk_pid.filter(|pid| state::alive(*pid));
    let served = server::serve(guard);
    // This process's leases and desk borrows lapse when it ends; free them
    // now.
    for lease in store.held_by(&wire.task) {
        if lease.pid == pid {
            store.release(&wire.task, &lease.window_id);
        }
    }
    desk::return_borrows(&store, &wire.task);
    served
}

/// The desktop driver a guard runs: Cua Driver when it is given and
/// starts, else agent-desktop (`assistant::driver`).
struct Driven {
    cua: Option<cua::Cua<child::AgentDesktop>>,
    agent: child::AgentDesktop,
    /// Cua Driver failed to start: agent-desktop serves from now on.
    fell_back: bool,
}

impl Driven {
    fn new(cua_driver: Option<&Path>, agent: child::AgentDesktop) -> Self {
        let backend = crate::assistant::driver::Driver::Cua.backend();
        Self {
            cua: cua_driver.map(|path| {
                cua::Cua::new(child::AgentDesktop::server(
                    path.to_path_buf(),
                    backend.mcp_args(None, "", false),
                    backend.env(),
                ))
            }),
            agent,
            fell_back: false,
        }
    }

    fn cua(&mut self) -> Option<&mut cua::Cua<child::AgentDesktop>> {
        if self.fell_back {
            return None;
        }
        self.cua.as_mut()
    }
}

impl child::Desktop for Driven {
    fn tools(&mut self) -> std::result::Result<Vec<serde_json::Value>, String> {
        if let Some(cua) = self.cua() {
            match cua.tools() {
                Ok(tools) => return Ok(tools),
                Err(why) => {
                    tracing::warn!(why, "Cua Driver didn't start; agent-desktop stands in");
                    self.fell_back = true;
                }
            }
        }
        self.agent.tools()
    }

    fn call(
        &mut self,
        tool: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<serde_json::Value, String> {
        match self.cua() {
            Some(cua) => cua.call(tool, args),
            None => self.agent.call(tool, args),
        }
    }
}

fn config(wire: &WireSpec) -> proxy::Config {
    proxy::Config {
        task: wire.task.clone(),
        lane: wire.lane,
        here_window: wire.here_window.clone().map(task_window),
        physical_input: wire.physical_input,
        blocked_apps: wire.blocked_apps.clone(),
        agent_display: wire.agent_display.clone(),
        autonomy: wire.autonomy,
        front_window: wire.front_window,
        open_apps: wire.open_apps || wire.front_window,
        observe_only: wire.observe_only,
    }
}

fn task_window(window: WireWindow) -> TaskWindow {
    TaskWindow {
        app: window.app,
        window_id: window.window_id,
    }
}

/// One window a task holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lease {
    pub task: ConversationId,
    pub window: TaskWindow,
    pub since_ms: u64,
}

/// Every live lease, for outlines and the task stack.
pub fn leases() -> Vec<Lease> {
    state::Store::standard()
        .map(|store| leases_in(&store))
        .unwrap_or_default()
}

fn leases_in(store: &state::Store) -> Vec<Lease> {
    store
        .leases()
        .into_iter()
        .map(|lease| Lease {
            task: lease.task,
            window: TaskWindow {
                app: lease.app,
                window_id: Some(lease.window_id),
            },
            since_ms: lease.since_ms,
        })
        .collect()
}

/// Gives up every lease `task` holds (it ended); its parked windows nobody
/// else holds go back where they were.
pub fn release_all(task: &str) {
    if let Some(store) = state::Store::standard() {
        store.release_all(task);
        desk::return_borrows(&store, task);
        // Accessibility may wait on a slow app; the task has ended.
        let _ = std::thread::Builder::new()
            .name("hex-put-back".into())
            .spawn(move || put_back_lapsed_in(&store));
    }
}

/// Notes that the person's Hand back (or Hex) parked window `window_id` of
/// `app`, which was at `from` (`parking`).
pub fn note_parked(window_id: &str, app: &str, from: Option<Rect>, how: ParkedHow) {
    if cfg!(test) {
        return;
    }
    if let Some(store) = state::Store::standard() {
        let owner = store
            .holder(window_id)
            .map(|lease| lease.task)
            .unwrap_or_default();
        let now = crate::desktop::events::now_ms();
        parking::park(&store, window_id, app, from, how, &owner, now);
    }
}

/// Takes window `window_id`'s parking record: it's going back to the
/// person (Take over, or it left its task).
pub fn take_parked(window_id: &str) -> Option<ParkedWindow> {
    if cfg!(test) {
        return None;
    }
    parking::unpark(&state::Store::standard()?, window_id)
}

fn put_back_lapsed_in(store: &state::Store) {
    if cfg!(test) {
        return;
    }
    let lapsed = parking::lapsed(store);
    if !lapsed.is_empty() {
        parking::put_back(store, &mut crate::assistant::space::Native, lapsed);
    }
}

/// Hex is quitting (an update, the person's Quit): every window it parked
/// goes back where it was, the desk's too. Waits at most `wait` for apps
/// that are slow to answer Accessibility; what's left goes back at the next
/// start.
pub fn put_back_all(wait: std::time::Duration) {
    if cfg!(test) {
        return;
    }
    let Some(store) = state::Store::standard() else {
        return;
    };
    let records = parking::all(&store);
    if records.is_empty() {
        return;
    }
    let (done, finished) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hex-put-back".into())
        .spawn(move || {
            parking::put_back(&store, &mut crate::assistant::space::Native, records);
            let _ = done.send(());
        });
    if spawned.is_ok() && finished.recv_timeout(wait).is_err() {
        tracing::warn!("parked windows are still going back as Hex quits");
    }
}

/// Pauses (taken over) or resumes (handed back) `task`'s desktop actions.
/// While paused its guard holds each action; the first result after a
/// resume tells it to look again.
pub fn set_paused(task: &str, paused: bool) {
    if let Some(store) = state::Store::standard() {
        store.set_paused(task, paused, crate::desktop::events::now_ms());
    }
}

/// Whether the person took `task` over (`set_paused`): its browser tool
/// waits too (`browser::proxy`).
pub fn paused(task: &str) -> bool {
    state::Store::standard().is_some_and(|store| store.paused(task))
}

/// When `task` was last handed back, as stored; it changes on each resume.
pub fn resumed(task: &str) -> Option<String> {
    state::Store::standard()?.resumed(task)
}

/// The task moving the real pointer or pressing real keys right now.
pub fn hands_off() -> Option<ConversationId> {
    state::Store::standard()?.hands_off()
}

/// Clears what tasks of an earlier run left behind: leases and hands-off
/// markers of processes that have ended, and pause markers; windows they
/// (or the desk of a Hex that quit, crashed, or was updated) left parked go
/// back where they were. Call once when the assistant starts (only while
/// `assistant::enabled`).
pub fn startup() {
    if let Some(store) = state::Store::standard() {
        store.sweep();
        store.clear_paused();
        put_back_lapsed_in(&store);
    }
    install_driver_in_background();
}

/// Cua Driver, when it is the chosen driver and not installed yet, comes in
/// the background (a pinned, checked download); until it is, agent-desktop
/// serves.
fn install_driver_in_background() {
    use crate::assistant::driver::{self, Driver};
    if cfg!(test) || !cfg!(target_os = "macos") || driver::chosen() != Driver::Cua {
        return;
    }
    if Driver::Cua.backend().find().is_some() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hex-cua-install".into())
        .spawn(
            || match driver::cua::install(&mut |step| tracing::info!(step, "Cua Driver")) {
                Ok(path) => tracing::info!(path = %path.display(), "Cua Driver is ready"),
                Err(error) => {
                    tracing::warn!(%error, "Cua Driver didn't install; agent-desktop serves")
                }
            },
        );
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't start installing Cua Driver");
    }
}

/// Whether a tool call named `tool` (as a backend reports it, e.g.
/// `mcp__agent-desktop__desktop_click`) works on the screen: `Some(true)`
/// when it acts there (clicks, types, presses, launches, opens, claims,
/// moves a window), `Some(false)` when it only reads or screenshots a
/// window, `None` for anything else (browsers, files, listings).
pub fn screen_work(tool: &str) -> Option<bool> {
    let name = tool.rsplit("__").next().unwrap_or(tool);
    let kind = policy::kind(name, &serde_json::Map::new());
    match kind {
        policy::Kind::Observe | policy::Kind::ReadWindow | policy::Kind::WindowShot => Some(false),
        policy::Kind::Claim => Some(true),
        kind if kind.acts() => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod screen_work_tests {
    #[test]
    fn desktop_actions_act_and_reads_look_and_the_rest_is_off_screen() {
        assert_eq!(
            super::screen_work("mcp__agent-desktop__desktop_click"),
            Some(true)
        );
        assert_eq!(super::screen_work("desktop_type"), Some(true));
        assert_eq!(
            super::screen_work("mcp__agent-desktop__hex_open_app"),
            Some(true)
        );
        assert_eq!(super::screen_work("desktop_claim_window"), Some(true));
        assert_eq!(
            super::screen_work("mcp__agent-desktop__desktop_snapshot"),
            Some(false)
        );
        assert_eq!(super::screen_work("desktop_list_windows"), None);
        assert_eq!(
            super::screen_work("mcp__agent-browser__agent_browser_open"),
            None
        );
        assert_eq!(super::screen_work("Write"), None);
    }
}
