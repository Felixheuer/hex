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
//!   with `here_physical_input`, or any task with `physical_input`; while one
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

pub use policy::CLAIM_TOOL;

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
    /// `assistant.background_front_window`: a Background task may claim
    /// the window in front of the person too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub front_window: bool,
    /// Reads only: every action is refused (a job's check, which must
    /// change nothing).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub observe_only: bool,
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
            state_dir: None,
            windows: WindowSource::default(),
            agent_display: None,
            autonomy: crate::assistant::autonomy::Autonomy::default(),
            front_window: spec.full_control,
            observe_only: false,
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
pub fn mcp_server(spec: &TaskSpec) -> Option<McpServer> {
    let desktop = crate::assistant::backends::mcp_server(&[])?;
    let hex = std::env::current_exe().ok()?;
    let store = state::Store::standard()?;
    Some(server_for(spec, &hex, &desktop, store.root()))
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
    wire.agent_desktop = Some(desktop.command.clone());
    wire.state_dir = Some(state_dir.to_path_buf());
    wire.agent_display = crate::assistant::computer::agent_display_setting();
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
    let desktop = child::AgentDesktop::new(program, flags);
    let pid = std::process::id();
    let guard = proxy::Guard::new(config(&wire), store.clone(), pid, desktop, screen);
    let served = server::serve(guard);
    // This process's leases lapse when it ends; free them now.
    for lease in store.held_by(&wire.task) {
        if lease.pid == pid {
            store.release(&wire.task, &lease.window_id);
        }
    }
    served
}

fn config(wire: &WireSpec) -> proxy::Config {
    proxy::Config {
        task: wire.task.clone(),
        lane: wire.lane,
        here_window: wire.here_window.as_ref().map(|window| TaskWindow {
            app: window.app.clone(),
            window_id: window.window_id.clone(),
        }),
        physical_input: wire.physical_input,
        blocked_apps: wire.blocked_apps.clone(),
        agent_display: wire.agent_display.clone(),
        autonomy: wire.autonomy,
        front_window: wire.front_window,
        observe_only: wire.observe_only,
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

/// Gives up every lease `task` holds (it ended).
pub fn release_all(task: &str) {
    if let Some(store) = state::Store::standard() {
        store.release_all(task);
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
/// markers of processes that have ended, and pause markers. Call once when
/// the assistant starts (only while `assistant::enabled`).
pub fn startup() {
    if let Some(store) = state::Store::standard() {
        store.sweep();
        store.clear_paused();
    }
}
