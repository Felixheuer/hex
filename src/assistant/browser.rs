//! Hidden browsers for web work: agent-browser (Vercel Labs), one session
//! per task, so twenty web tasks run at once without a window on the
//! person's screen and without waiting on the desktop's turn. Tasks get it
//! as a second tool beside the guard's desktop tools and are told to prefer
//! it for anything on the web.
//!
//! OWNER: browser.

use std::path::PathBuf;

use color_eyre::{Result, eyre::eyre};

use crate::assistant::backends::McpServer;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentBrowser {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Its own Chrome is downloaded (`agent-browser install`).
    pub browser_ready: bool,
}

pub fn agent_browser() -> AgentBrowser {
    AgentBrowser::default()
}

/// `brew install agent-browser`, then `agent-browser install`; blocks.
pub fn install() -> Result<()> {
    Err(eyre!("installing agent-browser isn't available yet"))
}

/// The browser tool for `task`: its own session, closed when the task ends.
pub fn mcp_server(task: &str) -> Option<McpServer> {
    let _ = task;
    None
}

/// Closes `task`'s browser session.
pub fn close(task: &str) {
    let _ = task;
}
