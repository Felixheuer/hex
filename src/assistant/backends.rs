//! Finding the agents installed on this Mac (Claude Code, Codex, OpenCode)
//! and agent-desktop, the desktop tool every task uses. A GUI app does not
//! inherit the person's shell PATH, so this looks where installers put them
//! and asks the login shell.
//!
//! OWNER: hands.

use std::path::PathBuf;

use color_eyre::{Result, eyre::eyre};

use crate::app_settings::AssistantBackend;

/// One agent found on this computer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Found {
    pub backend: AssistantBackend,
    /// The executable (`None` for `ClaudeApi`, and for OpenCode when only its
    /// service is running).
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Ready to take a task: installed and, as far as Hex can tell, signed in
    /// (Claude API: a key is saved; OpenCode: its service answers).
    pub ready: bool,
    /// Why it isn't ready, for a person.
    pub note: Option<String>,
}

/// Every backend, found or not, in `AssistantBackend` order.
pub fn detect() -> Vec<Found> {
    Vec::new()
}

/// The executable for `backend`, if installed.
pub fn executable(backend: AssistantBackend) -> Option<PathBuf> {
    let _ = backend;
    None
}

/// agent-desktop as found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentDesktop {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub accessibility: bool,
    pub screen_recording: bool,
}

pub fn agent_desktop() -> AgentDesktop {
    AgentDesktop::default()
}

/// Installs agent-desktop with Homebrew (`brew install agent-desktop`);
/// blocks until done. Fails with a person-readable reason when Homebrew is
/// missing.
pub fn install_agent_desktop() -> Result<()> {
    Err(eyre!("installing agent-desktop isn't available yet"))
}

/// Opens System Settings at agent-desktop's missing permission
/// (`agent-desktop permissions --guide`).
pub fn guide_permissions() {}
