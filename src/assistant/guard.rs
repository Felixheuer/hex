//! The guard between every task and the desktop: Hex's own MCP server
//! (`hex assistant mcp`), which each task's agent gets instead of
//! agent-desktop directly. It forwards agent-desktop's tools and enforces
//! what the lanes promise, so a prompt is never the only safeguard:
//!
//! - **Window leases.** A task acts only in windows it holds. The Here lane
//!   holds the window that was in front when asked; a Background task holds
//!   windows it opened (`launch`, new windows after its actions) or claimed
//!   while no other task holds them. Nobody acts in another task's window,
//!   and Background tasks never act in the window in front of the person.
//!   Leases live as files under `<support>/assistant/leases/`, created
//!   exclusively, so separate processes agree; a dead holder's lease lapses.
//! - **Input policy.** Real pointer and keys (`--headed`) only for a Here task
//!   with `here_physical_input`, or any task with `physical_input`; while one
//!   runs, `hands_off()` names it for the HUD.
//! - **Blocked apps.** No task reads or operates `blocked_apps`.
//! - **Take over.** While a task is paused its actions wait; when handed back
//!   its next result tells it to look again before acting.
//!
//! OWNER: guard.

use crate::assistant::backends::McpServer;
use crate::assistant::types::{ConversationId, Lane, TaskWindow};

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
}

/// The MCP server a task's agent should get in place of agent-desktop.
pub fn mcp_server(spec: &TaskSpec) -> Option<McpServer> {
    let _ = spec;
    None
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
    Vec::new()
}

/// Gives up every lease `task` holds (it ended).
pub fn release_all(task: &str) {
    let _ = task;
}

/// Pauses (taken over) or resumes (handed back) `task`'s desktop actions.
pub fn set_paused(task: &str, paused: bool) {
    let _ = (task, paused);
}

/// The task moving the real pointer or pressing real keys right now.
pub fn hands_off() -> Option<ConversationId> {
    None
}
