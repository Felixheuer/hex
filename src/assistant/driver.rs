//! The desktop driver: what reads apps' Accessibility trees and acts in
//! them for a task, behind Agent-Desk's guard. Two implement [`Backend`]:
//!
//! - **Cua Driver** (`cua-driver`, MIT, by Cua AI; the default): background
//!   computer use in the way Codex works: element actions through
//!   Accessibility, keys and clicks posted to the app's process, never the
//!   person's pointer or front window. Agent-Desk installs one pinned, checksummed,
//!   notarized release into its own folder ([`cua`]) and runs it as
//!   `cua-driver mcp --direct` inside the guard's process tree, so it acts
//!   with Agent-Desk's own Accessibility and Screen Recording grants, with its
//!   telemetry and update checks off.
//! - **agent-desktop** (`agent-desktop mcp`): the earlier driver, kept as
//!   the automatic fallback when Cua Driver is missing or fails to start.
//!
//! The guard serves the same `desktop_*` tools whichever runs
//! (`guard::cua` adapts Cua Driver's), so prompts, the overlay, and the
//! checks don't depend on which; the MCP server is named `desktop`
//! (`backends::MCP_NAME`). `assistant.desktop_driver` picks one.
//!
//! OWNER: guard.

pub mod cua;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which desktop driver a task's guard runs.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Driver {
    /// Cua Driver (`cua-driver mcp --direct`).
    #[default]
    Cua,
    /// agent-desktop (`agent-desktop mcp`).
    AgentDesktop,
}

impl Driver {
    pub fn backend(self) -> &'static dyn Backend {
        match self {
            Self::Cua => &CuaDriver,
            Self::AgentDesktop => &AgentDesktopDriver,
        }
    }

    /// The other driver, the fallback.
    pub fn other(self) -> Self {
        match self {
            Self::Cua => Self::AgentDesktop,
            Self::AgentDesktop => Self::Cua,
        }
    }
}

/// One desktop driver.
pub trait Backend: Sync {
    fn driver(&self) -> Driver;
    /// Its name as the person reads it.
    fn name(&self) -> &'static str;
    /// Its executable, when installed.
    fn find(&self) -> Option<PathBuf>;
    /// The arguments that start its MCP server for a task (session, agent
    /// id, real input allowed).
    fn mcp_args(&self, session: Option<&str>, agent_id: &str, headed: bool) -> Vec<String>;
    /// Environment its process gets beside Agent-Desk's agent environment.
    fn env(&self) -> Vec<(String, String)>;
    /// Whether it keeps a multi-agent session with its own cursors
    /// (`service::session`); Agent-Desk's overlay shows where a task acts either way.
    fn sessions(&self) -> bool;
    /// What the prompt says about the desktop tools.
    fn intro(&self) -> &'static str;
    /// Installs it; blocks (a download, at most minutes).
    fn install(&self, progress: &mut dyn FnMut(&str)) -> color_eyre::Result<()>;
}

/// The driver installed: which, and its executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Found {
    pub driver: Driver,
    pub path: PathBuf,
}

/// The driver the person's settings pick.
pub fn chosen() -> Driver {
    if cfg!(test) {
        return Driver::default();
    }
    crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.desktop_driver)
        .unwrap_or_default()
}

/// The driver to run: the chosen one when installed, else the other.
pub fn active() -> Option<Found> {
    pick(chosen(), |driver| driver.backend().find())
}

/// [`active`] with what is installed given (tests).
pub fn pick(chosen: Driver, find: impl Fn(Driver) -> Option<PathBuf>) -> Option<Found> {
    [chosen, chosen.other()]
        .into_iter()
        .find_map(|driver| find(driver).map(|path| Found { driver, path }))
}

/// Cua Driver, pinned ([`cua::VERSION`]).
pub struct CuaDriver;

impl Backend for CuaDriver {
    fn driver(&self) -> Driver {
        Driver::Cua
    }

    fn name(&self) -> &'static str {
        "Cua Driver"
    }

    fn find(&self) -> Option<PathBuf> {
        cua::installed()
    }

    fn mcp_args(&self, _session: Option<&str>, _agent_id: &str, _headed: bool) -> Vec<String> {
        // `--direct`: the driver runs in this process tree, so it acts with
        // Agent-Desk's grants; no daemon, no app of its own to grant.
        vec!["mcp".into(), "--direct".into()]
    }

    fn env(&self) -> Vec<(String, String)> {
        cua::env()
    }

    fn sessions(&self) -> bool {
        false
    }

    fn intro(&self) -> &'static str {
        "You can see and use the {computer}'s apps through the desktop tools (desktop_list_windows, \
         desktop_snapshot, desktop_click, desktop_type, …): a snapshot lists a window's elements \
         with refs (@<snapshot>:eN); act on a ref. They work in the background: windows stay \
         where they are, and nothing moves the person's pointer or takes their front window.\n"
    }

    fn install(&self, progress: &mut dyn FnMut(&str)) -> color_eyre::Result<()> {
        cua::install(progress).map(|_| ())
    }
}

/// agent-desktop, from Homebrew or npm.
pub struct AgentDesktopDriver;

impl Backend for AgentDesktopDriver {
    fn driver(&self) -> Driver {
        Driver::AgentDesktop
    }

    fn name(&self) -> &'static str {
        "agent-desktop"
    }

    fn find(&self) -> Option<PathBuf> {
        crate::assistant::backends::search::find("agent-desktop")
    }

    fn mcp_args(&self, session: Option<&str>, agent_id: &str, headed: bool) -> Vec<String> {
        let mut args = vec!["mcp".to_owned()];
        args.extend(crate::assistant::provider::flags(session, agent_id, headed));
        args
    }

    fn env(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn sessions(&self) -> bool {
        true
    }

    fn intro(&self) -> &'static str {
        "You can see and use the {computer}'s apps through the desktop tools (desktop_snapshot, \
         desktop_click, desktop_list_apps, …). The guide below shows them as shell commands: \
         each command is the tool of the same name (`agent-desktop list-apps` is \
         desktop_list_apps) and its flags are the tool's arguments. Always call the tools; never \
         run agent-desktop in a shell. Your session and agent id are already set.\n"
    }

    fn install(&self, progress: &mut dyn FnMut(&str)) -> color_eyre::Result<()> {
        crate::assistant::backends::install_tool_agent_desktop(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chosen_driver_runs_and_the_other_stands_in() {
        let both = |driver: Driver| Some(PathBuf::from(format!("/{driver:?}")));
        assert_eq!(pick(Driver::Cua, both).unwrap().driver, Driver::Cua);
        assert_eq!(
            pick(Driver::AgentDesktop, both).unwrap().driver,
            Driver::AgentDesktop
        );
        // Cua Driver missing (or not installed yet): agent-desktop.
        let only_agent = |driver: Driver| {
            (driver == Driver::AgentDesktop)
                .then(|| PathBuf::from("/opt/homebrew/bin/agent-desktop"))
        };
        assert_eq!(
            pick(Driver::Cua, only_agent).unwrap().driver,
            Driver::AgentDesktop
        );
        assert_eq!(pick(Driver::Cua, |_| None), None);
    }

    #[test]
    fn cua_driver_runs_in_hex_s_process_tree_without_sessions() {
        let args = CuaDriver.mcp_args(Some("s1"), "hex-1", true);
        assert_eq!(args, ["mcp", "--direct"]);
        assert!(!CuaDriver.sessions());
        let agent = AgentDesktopDriver.mcp_args(Some("s1"), "hex-1", false);
        assert_eq!(agent[0], "mcp");
        assert!(agent.contains(&"--session".to_owned()));
        assert!(CuaDriver.intro().contains("{computer}"));
        assert!(AgentDesktopDriver.intro().contains("{computer}"));
    }
}
