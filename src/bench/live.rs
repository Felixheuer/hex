//! The real [`App`] and [`Desktop`]: the running Agent-Desk (Dev or Live) through
//! its owner-only local API (`assistant-submit`, `status`) and the
//! conversation and audit files it writes; the desktop through
//! `lsappinfo` and agent-desktop's read-only `list-apps` / `list-windows`.
//! Nothing here clicks, types, or brings a window forward.

use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::{Result, eyre::bail, eyre::eyre};
use serde_json::Value;

use super::check::{ToolCall, Win};
use super::run::{App, Conv, Desktop};
use crate::assistant::store::Store;
use crate::assistant::types::{Part, Role};
use crate::control::{ControlCommand, ControlReply, ControlRequest};

/// The Agent-Desk a run talks to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Target {
    Dev,
    Live,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Live => "live",
        }
    }

    pub fn app_name(self) -> &'static str {
        match self {
            Self::Dev => "Agent-Desk Dev",
            Self::Live => "Agent-Desk",
        }
    }

    /// Its data folder (`AGENT_DESK_BENCH_SUPPORT_DIR` overrides it).
    pub fn support_dir(self) -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os("AGENT_DESK_BENCH_SUPPORT_DIR") {
            return Ok(dir.into());
        }
        let data = dirs::data_dir().ok_or_else(|| eyre!("no Application Support folder"))?;
        Ok(data.join(match self {
            Self::Dev => crate::app_paths::Variant::Dev.folder(),
            Self::Live => crate::app_paths::Variant::Live.folder(),
        }))
    }
}

pub struct LiveApp {
    support: PathBuf,
}

impl LiveApp {
    pub fn new(support: PathBuf) -> Self {
        Self { support }
    }

    fn call(&self, command: ControlCommand) -> Result<ControlReply> {
        let response =
            crate::local_api::call_control_in(&self.support, &ControlRequest::new(command))?;
        Ok(response.reply)
    }

    fn store(&self) -> Store {
        Store::at(self.conversations_dir())
    }

    fn conversations_dir(&self) -> PathBuf {
        self.support.join("assistant").join("conversations")
    }
}

/// A serde enum's snake_case name.
fn name_of(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl App for LiveApp {
    fn version(&mut self) -> Result<String> {
        match self.call(ControlCommand::Status)? {
            ControlReply::Status(report) => Ok(report.app_version),
            ControlReply::Error { message, .. } => bail!("{message}"),
            other => bail!("unexpected reply to status: {other:?}"),
        }
    }

    fn submit(&mut self, text: &str, conversation: Option<&str>) -> Result<String> {
        match self.call(ControlCommand::AssistantSubmit {
            text: text.to_owned(),
            conversation: conversation.map(str::to_owned),
        })? {
            ControlReply::Done { message, .. } if valid_id(message.trim()) => {
                Ok(message.trim().to_owned())
            }
            ControlReply::Error { code, message } => bail!("{code}: {message}"),
            other => bail!("the app doesn't take bench tasks (update it): {other:?}"),
        }
    }

    fn conversation(&mut self, id: &str) -> Option<Conv> {
        let stored = self.store().load(id)?;
        let mut conv = Conv {
            outcome: name_of(stored.current_outcome()),
            origin: name_of(stored.conversation.origin),
            workspace: stored.workspace.clone(),
            ..Conv::default()
        };
        for message in &stored.conversation.messages {
            match message.role {
                Role::User => {}
                Role::Assistant => {
                    conv.replies.push(message.at_ms);
                    for part in &message.parts {
                        match part {
                            Part::Text { text } if !text.trim().is_empty() => {
                                conv.answers.push((message.at_ms, text.clone()));
                            }
                            Part::Action { description } => conv.actions.push(description.clone()),
                            _ => {}
                        }
                    }
                }
            }
        }
        Some(conv)
    }

    fn created_since(&mut self, since_ms: u64) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.conversations_dir()) else {
            return Vec::new();
        };
        let since = std::time::UNIX_EPOCH + std::time::Duration::from_millis(since_ms);
        let store = self.store();
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|modified| modified + std::time::Duration::from_secs(1) >= since)
            })
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                let stored = store.load(id)?;
                (stored.conversation.created_ms >= since_ms).then(|| id.to_owned())
            })
            .collect()
    }

    fn tool_calls(&mut self, id: &str) -> Vec<ToolCall> {
        if !valid_id(id) {
            return Vec::new();
        }
        let path = self
            .support
            .join("assistant")
            .join("audit")
            .join(format!("{id}.jsonl"));
        read_audit(&path)
    }

    fn stop(&mut self, id: &str) {
        // The same request `agent-desk assistant stop-all` makes
        // (`assistant::service::remote`), in the target's data folder: the
        // app acts on it once and clears it.
        if !valid_id(id) {
            return;
        }
        let dir = self.support.join("assistant").join("requests");
        if crate::app_paths::create_private_dir(&dir).is_ok() {
            let nonce = crate::assistant::store::new_id();
            let _ = std::fs::write(dir.join(format!("{id}.stop.{nonce}")), b"");
        }
    }
}

/// An audit log's calls, without a job's own checks.
pub fn read_audit(path: &Path) -> Vec<ToolCall> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<crate::assistant::audit::Entry>(line).ok())
        .filter(|entry| !entry.check)
        .map(|entry| ToolCall {
            tool: entry.tool,
            input: entry.input,
        })
        .collect()
}

/// The Mac's desktop, read only.
pub struct MacDesktop {
    agent_desktop: Option<PathBuf>,
}

impl MacDesktop {
    pub fn new() -> Self {
        Self {
            agent_desktop: find("agent-desktop"),
        }
    }

    fn call(&self, args: &[&str]) -> Option<Value> {
        let output = Command::new(self.agent_desktop.as_ref()?)
            .args(args)
            .output()
            .ok()?;
        envelope(&String::from_utf8_lossy(&output.stdout))
    }
}

fn find(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()])
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// agent-desktop's reply data (the last `{"ok": true, …}` line).
pub fn envelope(output: &str) -> Option<Value> {
    output.lines().rev().find_map(|line| {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        (value.get("ok")? == &Value::Bool(true)).then(|| value.get("data").cloned())?
    })
}

pub fn apps_of(data: &Value) -> Vec<String> {
    data.get("apps")
        .and_then(Value::as_array)
        .map(|apps| {
            apps.iter()
                .filter_map(|app| app.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub fn windows_of(data: &Value) -> Vec<Win> {
    data.as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Win {
                    app: item
                        .get("app_name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    title: item
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    visible: item.get("visible").and_then(Value::as_bool).unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `lsappinfo info`'s `"LSDisplayName"="Name"`.
pub fn display_name(output: &str) -> Option<String> {
    let value = output.split_once('=')?.1.trim();
    let name = value.trim_matches('"');
    (!name.is_empty() && name != "[ NULL ]").then(|| name.to_owned())
}

impl Desktop for MacDesktop {
    fn front_app(&mut self) -> Option<String> {
        let front = Command::new("lsappinfo").arg("front").output().ok()?;
        let asn = String::from_utf8_lossy(&front.stdout).trim().to_owned();
        if asn.is_empty() {
            return None;
        }
        let info = Command::new("lsappinfo")
            .args(["info", "-only", "name", &asn])
            .output()
            .ok()?;
        display_name(&String::from_utf8_lossy(&info.stdout))
    }

    fn apps(&mut self) -> Option<Vec<String>> {
        self.call(&["list-apps"]).map(|data| apps_of(&data))
    }

    fn windows(&mut self, app: &str) -> Option<Vec<Win>> {
        self.call(&["list-windows", "--app", app, "--timeout-ms", "3000"])
            .map(|data| windows_of(&data))
    }
}
