//! The desktop driver: what reads apps' Accessibility trees and acts in
//! them for a task, behind Agent-Desk's guard. It is **Cua Driver**
//! (`cua-driver`, MIT, by Cua AI): background computer use in the way Codex
//! works: element actions through Accessibility, keys and clicks posted to
//! the app's process, never the person's pointer or front window.
//!
//! Agent-Desk installs a checksummed, notarized release into its own folder
//! ([`cua`]), keeps it current ([`update`]: a daily check of Cua's releases,
//! verified, self-tested, switched atomically, the previous one kept for a
//! rollback), and runs it as `cua-driver mcp --direct` inside the guard's
//! process tree, so it acts with Agent-Desk's own Accessibility and Screen
//! Recording grants, with its telemetry and update checks off.
//!
//! The guard serves its tools as `desktop_*` (`guard::cua` adapts them), so
//! prompts, the overlay, and the checks speak one vocabulary; the MCP server
//! is named `desktop` (`backends::MCP_NAME`).
//!
//! OWNER: guard.

pub mod cua;
pub mod update;

use std::path::PathBuf;

/// The driver's name as the person reads it.
pub const NAME: &str = "Cua Driver";

/// Cua Driver's executable, when installed (the current version).
pub fn find() -> Option<PathBuf> {
    cua::installed()
}

/// The arguments that start its MCP server for a task. `--direct`: the
/// driver runs in this process tree, so it acts with Agent-Desk's grants; no
/// daemon, no app of its own to grant.
pub fn mcp_args() -> Vec<String> {
    vec!["mcp".into(), "--direct".into()]
}

/// Environment its process gets beside Agent-Desk's agent environment.
pub fn env() -> Vec<(String, String)> {
    cua::env()
}

/// What the prompt says about the desktop tools (`{computer}` is "Mac").
pub const INTRO: &str = "You can see and use the {computer}'s apps through the desktop tools \
     (desktop_list_windows, desktop_snapshot, desktop_click, desktop_type, …): a snapshot lists a \
     window's elements with refs (@<snapshot>:eN); act on a ref. They work in the background: \
     windows stay where they are, and nothing moves the person's pointer or takes their front \
     window.\n";

/// Installs Cua Driver when it isn't; blocks (a download, at most minutes).
pub fn install(progress: &mut dyn FnMut(&str)) -> color_eyre::Result<PathBuf> {
    cua::install(progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cua_driver_runs_in_agent_desk_s_process_tree() {
        assert_eq!(mcp_args(), ["mcp", "--direct"]);
        assert!(INTRO.contains("{computer}"));
        assert!(
            env()
                .iter()
                .any(|(key, value)| key == "CUA_DRIVER_RS_UPDATE_CHECK" && value == "0")
        );
    }
}
