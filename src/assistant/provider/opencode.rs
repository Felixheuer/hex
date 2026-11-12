//! OpenCode: `opencode run --standalone --format json`, with the person's
//! own OpenCode configuration and sign-ins.
//!
//! Why the CLI and not the running service: Hex's other features only talk
//! to an already running OpenCode service, but that service loads its MCP
//! servers once, from its own configuration, and its API has no way to give
//! one session its own agent-desktop with this task's agent id and session.
//! `--standalone` runs a private server for just this task, and
//! `OPENCODE_CONFIG_CONTENT` adds to the person's configuration an agent
//! `hex` (Hex's instructions; shell and file edits denied) and agent-desktop
//! as a local MCP server. `--auto` approves the rest, so nothing waits on a
//! prompt; `--archived` keeps these sessions out of the person's list.
//! A conversation continues with `--session <id>`.

use color_eyre::{Result, eyre::eyre};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin, WITHHELD_TOOLS};
use crate::app_settings::AssistantBackend;

const AGENT: &str = "hex";

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let executable = super::executable(AssistantBackend::OpenCode)?;
    let mut command = super::command(&executable, &run.workdir, run.desktop.as_ref());
    command.env("OPENCODE_CONFIG_CONTENT", config(run).to_string());
    command.args(arguments(run));
    let mut process = Process::spawn(command, Stdin::Null, control)?;
    let mut stream = Stream::default();
    while let Some(line) = process.next_line() {
        stream.line(&line, report);
    }
    let (status, stderr) = process.finish(control);
    if control.cancelled() {
        return Err(eyre!("cancelled"));
    }
    stream.finish().unwrap_or_else(|| {
        Err(super::exit_error(
            AssistantBackend::OpenCode,
            status,
            &stderr,
        ))
    })
}

fn arguments(run: &Run) -> Vec<String> {
    let mut args: Vec<String> = [
        "run",
        "--standalone",
        "--format",
        "json",
        "--auto",
        "--archived",
        "--agent",
        AGENT,
    ]
    .map(str::to_owned)
    .into();
    if let Some(session) = &run.resume {
        args.extend(["--session".into(), session.clone()]);
    }
    // A private server doesn't know the person's model; without one it
    // falls back to a free model and its limits.
    if let Some(model) = run
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .or_else(crate::assistant::backends::opencode_recent_model)
    {
        args.extend(["--model".into(), model]);
    }
    if let Some(image) = &run.image {
        args.extend(["--file".into(), image.display().to_string()]);
    }
    args.extend(["--".into(), run.prompt.clone()]);
    args
}

/// What `OPENCODE_CONFIG_CONTENT` adds to the person's configuration.
fn config(run: &Run) -> Value {
    let mut tools = Map::new();
    for tool in WITHHELD_TOOLS {
        tools.insert(format!("agent-desktop_{tool}"), Value::Bool(false));
    }
    let mut config = json!({"agent": {AGENT: {
        "mode": "primary",
        "description": "Hex's assistant",
        "prompt": run.system,
        "permission": {"bash": "deny", "edit": "deny", "webfetch": "allow"},
        "tools": tools,
    }}});
    if let Some(desktop) = &run.desktop {
        config["mcp"] = desktop.server.opencode_config();
    }
    config
}

/// `opencode run --format json` events, read line by line.
#[derive(Default)]
pub(super) struct Stream {
    describer: Describer,
    session: Option<String>,
    /// The text written since the last tool call: the answer so far.
    answer: String,
    wrote: bool,
    tool_after_text: bool,
    error: Option<String>,
}

impl Stream {
    pub(super) fn line(&mut self, line: &str, report: &mut dyn FnMut(Report)) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if self.session.is_none()
            && let Some(session) = event.get("sessionID").and_then(Value::as_str)
        {
            self.session = Some(session.to_owned());
            report(Report::Session(session.to_owned()));
        }
        let part = event.get("part").unwrap_or(&Value::Null);
        match event.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                if text.trim().is_empty() {
                    return;
                }
                if self.tool_after_text {
                    self.tool_after_text = false;
                    report(Report::Delta("\n\n".into()));
                }
                if !self.answer.is_empty() {
                    self.answer.push_str("\n\n");
                }
                self.answer.push_str(text.trim());
                self.wrote = true;
                report(Report::Delta(text.trim().to_owned()));
            }
            Some("tool_use") => {
                self.answer.clear();
                if self.wrote {
                    self.tool_after_text = true;
                }
                for (tool, args) in calls(part) {
                    for item in self.describer.call(&tool, &args) {
                        report(item);
                    }
                    if let Some(output) = part.pointer("/state/output").and_then(Value::as_str) {
                        for item in self.describer.result(&tool, output) {
                            report(item);
                        }
                    }
                }
            }
            Some("error") => {
                let why = event
                    .pointer("/error/data/message")
                    .or_else(|| event.pointer("/error/message"))
                    .or_else(|| event.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        event.get("error").map(Value::to_string).unwrap_or_default()
                    });
                self.error = Some(why);
            }
            _ => {}
        }
    }

    pub(super) fn finish(self) -> Option<Result<Finished>> {
        if let Some(why) = self.error {
            return Some(Err(eyre!("OpenCode: {why}")));
        }
        (!self.answer.is_empty()).then(|| {
            Ok(Finished {
                text: self.answer.trim().to_owned(),
            })
        })
    }
}

/// The desktop tools one tool part called: itself, or, for OpenCode's code
/// mode (`execute`), each `tools["agent-desktop"].desktop_*` call in its code.
fn calls(part: &Value) -> Vec<(String, Value)> {
    let tool = part.get("tool").and_then(Value::as_str).unwrap_or_default();
    let input = part.pointer("/state/input").cloned().unwrap_or(Value::Null);
    if tool != "execute" {
        return vec![(tool.to_owned(), input)];
    }
    let code = input
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let in_code = code_calls(code);
    let recorded = part
        .pointer("/state/metadata/metadata/toolCalls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let tool = call.get("tool").and_then(Value::as_str)?;
                    let name = tool.strip_prefix("agent-desktop.")?;
                    let args = call.get("input").cloned().or_else(|| {
                        in_code
                            .iter()
                            .find(|(called, _)| called == name)
                            .map(|(_, args)| args.clone())
                    });
                    Some((name.to_owned(), args.unwrap_or(Value::Null)))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if recorded.is_empty() {
        in_code
    } else {
        recorded
    }
}

/// `desktop_*(…)` calls in code-mode JavaScript, with their object argument
/// read as well as a quick look can.
fn code_calls(code: &str) -> Vec<(String, Value)> {
    let call = Regex::new(r"(desktop_[a-z_]+)\s*\(\s*(\{[^()]*\})?").expect("valid pattern");
    let field = Regex::new(
        r#"["']?(\w+)["']?\s*:\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|-?\d+(?:\.\d+)?|true|false)"#,
    )
    .expect("valid pattern");
    call.captures_iter(code)
        .map(|captures| {
            let name = captures[1].to_owned();
            let args = captures.get(2).map_or(Value::Null, |object| {
                serde_json::from_str(object.as_str()).unwrap_or_else(|_| {
                    let mut map = Map::new();
                    for field in field.captures_iter(object.as_str()) {
                        let raw = &field[2];
                        let value = if raw.starts_with('\'') {
                            Value::String(raw.trim_matches('\'').to_owned())
                        } else {
                            serde_json::from_str(raw).unwrap_or(Value::Null)
                        };
                        map.insert(field[1].to_owned(), value);
                    }
                    Value::Object(map)
                })
            });
            (name, args)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_desktop_task_reports_its_steps_and_final_text() {
        let mut stream = Stream::default();
        let mut reports = Vec::new();
        for line in include_str!("../../../tests/fixtures/assistant/opencode-desktop.jsonl").lines()
        {
            stream.line(line, &mut |report| reports.push(report));
        }
        assert_eq!(
            reports[0],
            Report::Session("ses_e30d7e935ffeGZdt4fv6EXRfWH".into())
        );
        assert!(reports.contains(&Report::Step {
            doing: "Checking open apps".into(),
            done: "Checked open apps".into()
        }));
        assert_eq!(stream.finish().unwrap().unwrap().text, "69");
    }

    #[test]
    fn code_mode_calls_are_read_from_their_code() {
        let calls = code_calls(
            r#"const s = await tools["agent-desktop"].desktop_snapshot({ app: 'Mail', interactive_only: true });
               await tools["agent-desktop"].desktop_click({"ref_id": "@s1:e2"});"#,
        );
        assert_eq!(calls[0].0, "desktop_snapshot");
        assert_eq!(calls[0].1, json!({"app": "Mail", "interactive_only": true}));
        assert_eq!(
            calls[1],
            ("desktop_click".into(), json!({"ref_id": "@s1:e2"}))
        );
    }

    #[test]
    fn an_error_event_fails_the_task() {
        let mut stream = Stream::default();
        stream.line(
            r#"{"type":"error","sessionID":"ses_1","error":{"name":"ProviderAuthError","data":{"message":"no credentials"}}}"#,
            &mut |_| {},
        );
        let error = stream.finish().unwrap().unwrap_err().to_string();
        assert_eq!(error, "OpenCode: no credentials");
    }
}
