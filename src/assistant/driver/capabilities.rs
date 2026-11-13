//! What the installed Cua Driver offers, checked after each update (and once
//! when no check is kept): its tools (MCP `tools/list`), diffed against the
//! version before, so a new tool is logged and shown in `agent-desk
//! assistant driver status` with whether the guard serves it yet
//! (`guard::driver_tools_used`). Kept in `<cua-driver>/capabilities.json`.

use serde::{Deserialize, Serialize};

use super::cua::Home;

/// The last capability check.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Capabilities {
    /// The version checked.
    pub version: String,
    /// Its tools, sorted.
    pub tools: Vec<String>,
    /// The version checked before it, when there was one.
    #[serde(default)]
    pub before: Option<String>,
    /// Tools it has that the version before didn't.
    #[serde(default)]
    pub added: Vec<String>,
    /// Tools the version before had that it hasn't.
    #[serde(default)]
    pub removed: Vec<String>,
    pub checked_ms: u64,
    /// Whether its agent cursor can show as tasks run it (`mcp --direct`
    /// has no overlay host); `None` when it couldn't be told.
    #[serde(default)]
    pub cursor: Option<bool>,
}

/// What a check reads from a driver.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub tools: Vec<String>,
    pub cursor: Option<bool>,
}

impl Capabilities {
    /// A check of `version` with `tools`, after `before` (the last one kept).
    pub fn after(
        before: Option<&Capabilities>,
        version: &str,
        mut tools: Vec<String>,
        now_ms: u64,
    ) -> Self {
        tools.sort();
        tools.dedup();
        let (added, removed) = match before {
            Some(before) => (
                tools
                    .iter()
                    .filter(|tool| !before.tools.contains(tool))
                    .cloned()
                    .collect(),
                before
                    .tools
                    .iter()
                    .filter(|tool| !tools.contains(tool))
                    .cloned()
                    .collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        // A check of the same version keeps what changed with it.
        if let Some(before) = before.filter(|before| before.version == version) {
            return Self {
                version: version.to_owned(),
                tools,
                before: before.before.clone(),
                added: before.added.clone(),
                removed: before.removed.clone(),
                checked_ms: now_ms,
                cursor: before.cursor,
            };
        }
        Self {
            version: version.to_owned(),
            tools,
            before: before.map(|before| before.version.clone()),
            added,
            removed,
            checked_ms: now_ms,
            cursor: None,
        }
    }

    /// Its tools the guard doesn't serve (yet), of `used`.
    pub fn unused(&self, used: &[&str]) -> Vec<String> {
        self.tools
            .iter()
            .filter(|tool| !used.contains(&tool.as_str()))
            .cloned()
            .collect()
    }

    /// Lines for `driver status`.
    pub fn lines(&self, used: &[&str]) -> Vec<String> {
        let mut lines = vec![format!(
            "tools: {} ({} served to tasks)",
            self.tools.len(),
            self.tools
                .iter()
                .filter(|tool| used.contains(&tool.as_str()))
                .count()
        )];
        if let Some(before) = &self.before
            && (!self.added.is_empty() || !self.removed.is_empty())
        {
            if !self.added.is_empty() {
                let served: Vec<String> = self
                    .added
                    .iter()
                    .map(|tool| {
                        if used.contains(&tool.as_str()) {
                            tool.clone()
                        } else {
                            format!("{tool} (not served yet)")
                        }
                    })
                    .collect();
                lines.push(format!("new since {before}: {}", served.join(", ")));
            }
            if !self.removed.is_empty() {
                lines.push(format!("gone since {before}: {}", self.removed.join(", ")));
            }
        }
        lines.push(
            match self.cursor {
                Some(true) => "agent cursor: Cua Driver's overlay shows where tasks act",
                Some(false) => {
                    "agent cursor: Cua Driver's overlay isn't available as tasks run it; the live \
                     pictures ring where tasks act"
                }
                None => "agent cursor: not checked",
            }
            .to_owned(),
        );
        lines
    }
}

fn path(home: &Home) -> std::path::PathBuf {
    home.path().join("capabilities.json")
}

/// The last check kept in `home`.
pub fn read(home: &Home) -> Option<Capabilities> {
    serde_json::from_str(&std::fs::read_to_string(path(home)).ok()?).ok()
}

fn save(home: &Home, capabilities: &Capabilities) {
    let saved = std::fs::create_dir_all(home.path()).and_then(|()| {
        let partial = home.path().join(".capabilities.json.partial");
        std::fs::write(
            &partial,
            serde_json::to_vec_pretty(capabilities).unwrap_or_default(),
        )?;
        std::fs::rename(&partial, path(home))
    });
    if let Err(error) = saved {
        tracing::warn!(%error, "couldn't keep Cua Driver's capabilities");
    }
}

/// Checks the installed version's tools when it wasn't yet (an update, or
/// no check kept), with `probe` (`guard::driver_probe`); logs what is
/// new. The check kept, or why it couldn't run.
pub fn refresh(
    home: &Home,
    probe: &dyn Fn(&std::path::Path) -> Result<Probe, String>,
    now_ms: u64,
) -> Result<Capabilities, String> {
    let version = home.current().ok_or("Cua Driver isn't installed")?;
    let before = read(home);
    if let Some(before) = before.as_ref().filter(|before| before.version == version) {
        return Ok(before.clone());
    }
    let executable = home.installed().ok_or("Cua Driver isn't installed")?;
    let probed = probe(&executable)?;
    let mut checked = Capabilities::after(before.as_ref(), &version, probed.tools, now_ms);
    checked.cursor = probed.cursor;
    if !checked.added.is_empty() {
        let used = crate::assistant::guard::driver_tools_used();
        tracing::info!(
            version,
            new = ?checked.added,
            not_served = ?checked
                .added
                .iter()
                .filter(|tool| !used.contains(&tool.as_str()))
                .collect::<Vec<_>>(),
            "Cua Driver offers new tools"
        );
    }
    if !checked.removed.is_empty() {
        tracing::warn!(version, gone = ?checked.removed, "Cua Driver dropped tools");
    }
    save(home, &checked);
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_new_version_s_tools_are_diffed_against_the_last_check() {
        let first = Capabilities::after(None, "0.49.1", names(&["click", "list_windows"]), 1);
        assert!(first.added.is_empty() && first.before.is_none());
        let second = Capabilities::after(
            Some(&first),
            "0.49.3",
            names(&[
                "run_steps",
                "list_windows",
                "click",
                "verify_state",
                "list_windows",
            ]),
            2,
        );
        assert_eq!(second.before.as_deref(), Some("0.49.1"));
        assert_eq!(second.added, ["run_steps", "verify_state"]);
        assert!(second.removed.is_empty());
        assert_eq!(
            second.tools,
            ["click", "list_windows", "run_steps", "verify_state"]
        );
        // A later check of the same version keeps what was new with it.
        let again = Capabilities::after(Some(&second), "0.49.3", second.tools.clone(), 3);
        assert_eq!(again.added, second.added);
        let lines = again.lines(&["click", "list_windows", "run_steps"]);
        assert_eq!(lines[0], "tools: 4 (3 served to tasks)");
        assert_eq!(
            lines[1],
            "new since 0.49.1: run_steps, verify_state (not served yet)"
        );
        assert_eq!(
            again.unused(&["click", "list_windows", "run_steps"]),
            ["verify_state"]
        );
        let dropped = Capabilities::after(Some(&again), "0.50.0", names(&["click"]), 4);
        assert_eq!(
            dropped.removed,
            ["list_windows", "run_steps", "verify_state"]
        );
    }

    #[test]
    fn a_check_runs_once_per_version_and_is_kept() {
        let dir = std::env::temp_dir().join(format!("hex-cua-caps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = Home(dir.clone());
        let version = dir.join("0.49.3").join("cua-driver-rs-0.49.3-darwin-arm64");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(version.join("cua-driver"), "").unwrap();
        std::fs::write(dir.join("current"), "0.49.3\n").unwrap();
        let calls = std::cell::Cell::new(0);
        let list = |_: &std::path::Path| {
            calls.set(calls.get() + 1);
            Ok(Probe {
                tools: names(&["click", "run_steps"]),
                cursor: Some(false),
            })
        };
        let checked = refresh(&home, &list, 5).unwrap();
        assert_eq!(checked.tools, ["click", "run_steps"]);
        assert_eq!(checked.cursor, Some(false));
        assert_eq!(read(&home), Some(checked));
        refresh(&home, &list, 6).unwrap();
        assert_eq!(calls.get(), 1, "the same version isn't listed again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
