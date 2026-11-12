//! Codex: `codex exec --json`, signed in with the person's ChatGPT account.
//!
//! The prompt goes in on stdin, Hex's instructions as `developer_instructions`,
//! and agent-desktop as an MCP server through `-c mcp_servers.agent-desktop.*`
//! with its tools approved up front (`default_tools_approval_mode`), because
//! `exec` never asks. Shell commands run in the read-only sandbox. A
//! conversation continues with `codex exec resume <thread id>`.

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin};
use crate::app_settings::AssistantBackend;

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let executable = super::executable(AssistantBackend::Codex)?;
    let mut command = super::command(&executable, &run.workdir, run.desktop.as_ref());
    command.args(arguments(run));
    let mut process = Process::spawn(
        command,
        Stdin::Write(run.prompt.clone().into_bytes()),
        control,
    )?;
    let mut stream = Stream::default();
    while let Some(line) = process.next_line() {
        stream.line(&line, report);
    }
    let (status, stderr) = process.finish(control);
    if control.cancelled() {
        return Err(eyre!("cancelled"));
    }
    stream
        .finish()
        .unwrap_or_else(|| Err(super::exit_error(AssistantBackend::Codex, status, &stderr)))
}

fn arguments(run: &Run) -> Vec<String> {
    let mut args: Vec<String> = vec!["exec".into()];
    if let Some(session) = &run.resume {
        args.extend(["resume".into(), session.clone()]);
    }
    if let Some(image) = &run.image {
        args.extend(["--image".into(), image.display().to_string()]);
    }
    args.extend(["--json".into(), "--skip-git-repo-check".into()]);
    // A job may write in its workspace (and reach the network, to install
    // Python packages); anything else reads only.
    let mut config = vec![
        r#"approval_policy="never""#.to_owned(),
        format!("developer_instructions={}", toml_string(&run.system)),
    ];
    match &run.workspace {
        Some(workspace) => {
            args.extend(["-C".into(), workspace.display().to_string()]);
            config.push(r#"sandbox_mode="workspace-write""#.into());
            config.push("sandbox_workspace_write.network_access=true".into());
        }
        None => config.push(r#"sandbox_mode="read-only""#.into()),
    }
    // Windows: the sandbox that needs no administrator setup (the elevated
    // one asks for it once, which `codex exec` can't).
    if cfg!(windows) {
        config.push(r#"windows.sandbox="unelevated""#.into());
    }
    for value in config {
        args.extend(["-c".into(), value]);
    }
    args.extend(super::servers::codex_args(run));
    if let Some(model) = run
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
    {
        args.extend(["--model".into(), model.trim().to_owned()]);
    }
    args.push("-".into());
    args
}

/// A TOML basic string: JSON's escapes are a subset of TOML's.
fn toml_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

/// `codex exec --json` events, read line by line.
#[derive(Default)]
pub(super) struct Stream {
    describer: Describer,
    last_message: Option<String>,
    outcome: Option<Result<Finished>>,
    error: Option<String>,
}

impl Stream {
    pub(super) fn line(&mut self, line: &str, report: &mut dyn FnMut(Report)) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let item = event.get("item").unwrap_or(&Value::Null);
        let item_type = item.get("type").and_then(Value::as_str);
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if let Some(thread) = event.get("thread_id").and_then(Value::as_str) {
                    report(Report::Session(thread.to_owned()));
                }
            }
            Some("item.started") if item_type == Some("mcp_tool_call") => {
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
                let args = item.get("arguments").cloned().unwrap_or(Value::Null);
                for item in self.describer.call(tool, &args) {
                    report(item);
                }
            }
            Some("item.completed") if item_type == Some("mcp_tool_call") => {
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
                if let Some(result) = item.get("result").filter(|result| !result.is_null()) {
                    for item in self.describer.result(tool, &result.to_string()) {
                        report(item);
                    }
                }
            }
            Some("item.completed") if item_type == Some("agent_message") => {
                let text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                if text.is_empty() {
                    return;
                }
                if self.last_message.is_some() {
                    report(Report::Delta("\n\n".into()));
                }
                report(Report::Delta(text.to_owned()));
                self.last_message = Some(text.to_owned());
            }
            Some("turn.completed") => {
                self.outcome = Some(match self.last_message.take() {
                    Some(text) => Ok(Finished { text }),
                    None => Err(eyre!("Codex finished without an answer")),
                });
            }
            Some("turn.failed") => {
                let why = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.error.take())
                    .unwrap_or_else(|| "the turn failed".into());
                self.outcome = Some(Err(eyre!("Codex: {why}")));
            }
            Some("error") => {
                self.error = event
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            _ => {}
        }
    }

    pub(super) fn finish(mut self) -> Option<Result<Finished>> {
        self.outcome
            .take()
            .or_else(|| self.error.take().map(|why| Err(eyre!("Codex: {why}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::provider::Desktop;

    #[test]
    fn a_recorded_desktop_task_reports_its_steps_and_last_message() {
        let mut stream = Stream::default();
        let mut reports = Vec::new();
        for line in include_str!("../../../tests/fixtures/assistant/codex-desktop.jsonl").lines() {
            stream.line(line, &mut |report| reports.push(report));
        }
        assert_eq!(
            reports[0],
            Report::Session("01a1cf29-93ae-7e90-b88a-e976026e8d5c".into())
        );
        assert!(reports.contains(&Report::Step {
            doing: "Checking open apps".into(),
            done: "Checked open apps".into()
        }));
        assert_eq!(
            stream.finish().unwrap().unwrap().text,
            "- Citrix Viewer\n- Finder\n- Ghostty"
        );
    }

    #[test]
    fn a_failed_turn_fails_with_its_reason() {
        let mut stream = Stream::default();
        stream.line(
            r#"{"type":"error","message":"stream disconnected"}"#,
            &mut |_| {},
        );
        stream.line(
            r#"{"type":"turn.failed","error":{"message":"usage limit reached"}}"#,
            &mut |_| {},
        );
        let error = stream.finish().unwrap().unwrap_err().to_string();
        assert_eq!(error, "Codex: usage limit reached");
    }

    #[test]
    fn arguments_resume_and_approve_agent_desktop() {
        let run = Run {
            system: "Line one\n\"quoted\"".into(),
            image: Some("/tmp/shot.png".into()),
            resume: Some("thread-1".into()),
            desktop: Some(Desktop::fixture(None, "hex-1")),
            ..Run::fixture(AssistantBackend::Codex)
        };
        let args = arguments(&run);
        assert_eq!(
            &args[..5],
            ["exec", "resume", "thread-1", "--image", "/tmp/shot.png"]
        );
        assert_eq!(args.last().unwrap(), "-");
        assert!(args.contains(&r#"developer_instructions="Line one\n\"quoted\"""#.to_owned()));
        assert!(args.contains(&r#"mcp_servers.agent-desktop.args=["mcp"]"#.to_owned()));
        assert!(args.iter().any(
            |arg| arg.starts_with("mcp_servers.agent-desktop.enabled_tools=")
                && arg.contains("desktop_click")
                && !arg.contains("desktop_session_end")
        ));
        assert!(args.contains(
            &r#"mcp_servers.agent-desktop.default_tools_approval_mode="approve""#.to_owned()
        ));
    }
}
