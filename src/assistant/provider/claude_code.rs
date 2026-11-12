//! Claude Code: `claude -p` with stream-json in and out, signed in with the
//! person's own account.
//!
//! The question (and a quick question's screenshot, as an image block) goes
//! in on stdin as one stream-json user message. Only agent-desktop's MCP
//! server is loaded (`--strict-mcp-config`), the built-in tools are cut to web
//! search and fetch, and `dontAsk` denies anything not allowed up front, so a
//! task never waits on a permission prompt. Hooks are off: the person's own
//! hooks are for their coding sessions, not this.

use std::collections::HashMap;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin, WITHHELD_TOOLS};
use crate::app_settings::AssistantBackend;

const SERVER: &str = crate::assistant::backends::MCP_NAME;

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let executable = super::executable(AssistantBackend::ClaudeCode)?;
    let mut command = super::command(&executable, &run.workdir, run.desktop.as_ref());
    command.args(arguments(run));
    let input = input(run)?;
    let mut process = Process::spawn(command, Stdin::Write(input), control)?;
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
            AssistantBackend::ClaudeCode,
            status,
            &stderr,
        ))
    })
}

fn arguments(run: &Run) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "dontAsk",
        "--settings",
        r#"{"disableAllHooks":true}"#,
        "--strict-mcp-config",
        "--tools",
        "WebSearch,WebFetch",
    ]
    .map(str::to_owned)
    .into();
    args.extend(["--system-prompt".into(), run.system.clone()]);
    let mut allowed = vec!["WebSearch".to_owned(), "WebFetch".to_owned()];
    if let Some(desktop) = &run.desktop {
        args.extend(["--mcp-config".into(), desktop.server.claude_config()]);
        allowed.push(format!("mcp__{SERVER}"));
        let withheld: Vec<String> = WITHHELD_TOOLS
            .iter()
            .map(|tool| format!("mcp__{SERVER}__{tool}"))
            .collect();
        args.extend(["--disallowedTools".into(), withheld.join(",")]);
    }
    args.extend(["--allowedTools".into(), allowed.join(",")]);
    if let Some(model) = run
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
    {
        args.extend(["--model".into(), model.trim().to_owned()]);
    }
    if let Some(session) = &run.resume {
        args.extend(["--resume".into(), session.clone()]);
    }
    args
}

/// The stream-json user message: the prompt, and the screenshot inline.
fn input(run: &Run) -> Result<Vec<u8>> {
    let mut content = vec![json!({"type": "text", "text": run.prompt})];
    if let Some(image) = &run.image {
        let png =
            std::fs::read(image).map_err(|error| eyre!("couldn't read the screenshot: {error}"))?;
        content.push(json!({"type": "image", "source": {
            "type": "base64", "media_type": "image/png", "data": super::base64(&png)}}));
    }
    let message = json!({"type": "user", "message": {"role": "user", "content": content}});
    let mut line = serde_json::to_vec(&message)?;
    line.push(b'\n');
    Ok(line)
}

/// Claude Code's stream-json output, read line by line.
#[derive(Default)]
pub(super) struct Stream {
    describer: Describer,
    /// Tool use id to tool name, to read each result.
    tools: HashMap<String, String>,
    /// Text has streamed, and a tool ran after it: the next text starts a
    /// new paragraph.
    wrote: bool,
    tool_after_text: bool,
    outcome: Option<Result<Finished>>,
}

impl Stream {
    pub(super) fn line(&mut self, line: &str, report: &mut dyn FnMut(Report)) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("system") if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                if let Some(session) = event.get("session_id").and_then(Value::as_str) {
                    report(Report::Session(session.to_owned()));
                }
            }
            Some("stream_event") => {
                let delta = event.pointer("/event/delta");
                if delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta")
                    && let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str)
                    && !text.is_empty()
                {
                    if self.tool_after_text {
                        self.tool_after_text = false;
                        report(Report::Delta("\n\n".into()));
                    }
                    self.wrote = true;
                    report(Report::Delta(text.to_owned()));
                }
            }
            Some("assistant") => {
                for block in blocks(&event) {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        self.tools.insert(id.to_owned(), name.to_owned());
                    }
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    for item in self.describer.call(name, &input) {
                        report(item);
                    }
                    if self.wrote {
                        self.tool_after_text = true;
                    }
                }
            }
            Some("user") => {
                for block in blocks(&event) {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(name) = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|id| self.tools.get(id))
                        .cloned()
                    else {
                        continue;
                    };
                    let output = match block.get("content") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => continue,
                    };
                    for item in self.describer.result(&name, &output) {
                        report(item);
                    }
                }
            }
            Some("result") => {
                let failed = event.get("is_error").and_then(Value::as_bool) == Some(true)
                    || event.get("subtype").and_then(Value::as_str) != Some("success");
                let text = event
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                self.outcome = Some(if failed {
                    let errors = event
                        .get("errors")
                        .and_then(Value::as_array)
                        .map(|errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                        .filter(|errors| !errors.is_empty());
                    let why = errors.unwrap_or(if text.is_empty() {
                        "it stopped without an answer".into()
                    } else {
                        text
                    });
                    Err(eyre!("Claude Code: {why}"))
                } else {
                    Ok(Finished { text })
                });
            }
            _ => {}
        }
    }

    /// The answer or error, when the stream ended with a result.
    pub(super) fn finish(self) -> Option<Result<Finished>> {
        self.outcome
    }
}

fn blocks(event: &Value) -> impl Iterator<Item = &Value> {
    event
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::provider::Desktop;
    use crate::assistant::types::TaskWindow;

    fn replay(fixture: &str) -> (Vec<Report>, Option<Result<Finished>>) {
        let mut stream = Stream::default();
        let mut reports = Vec::new();
        for line in fixture.lines() {
            stream.line(line, &mut |report| reports.push(report));
        }
        (reports, stream.finish())
    }

    #[test]
    fn a_recorded_desktop_task_streams_steps_text_and_the_answer() {
        let (reports, outcome) = replay(include_str!(
            "../../../tests/fixtures/assistant/claude-code-desktop.jsonl"
        ));
        assert_eq!(
            reports[0],
            Report::Session("066af6f5-1a71-4e44-bcd1-8cba86dd6c65".into())
        );
        assert!(reports.contains(&Report::Step {
            doing: "Checking open apps".into(),
            done: "Checked open apps".into()
        }));
        let text: String = reports
            .iter()
            .filter_map(|report| match report {
                Report::Delta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let answer = outcome.unwrap().unwrap().text;
        assert!(answer.starts_with("Drei der Apps"), "{answer}");
        assert_eq!(text, answer);
    }

    #[test]
    fn a_recorded_snapshot_and_click_name_the_window_and_element() {
        let (reports, outcome) = replay(include_str!(
            "../../../tests/fixtures/assistant/claude-code-click.jsonl"
        ));
        assert!(reports.contains(&Report::Window(TaskWindow {
            app: "TextEdit".into(),
            window_id: Some("w-42".into())
        })));
        assert!(reports.contains(&Report::Step {
            doing: "Clicking “Bold” in TextEdit".into(),
            done: "Clicked “Bold” in TextEdit".into()
        }));
        assert!(reports.contains(&Report::Delta("\n\n".into())));
        assert_eq!(outcome.unwrap().unwrap().text, "Done: the text is bold.");
    }

    #[test]
    fn an_error_result_fails_with_its_reason() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: x"]}"#;
        let (_, outcome) = replay(line);
        let error = outcome.unwrap().unwrap_err().to_string();
        assert_eq!(
            error,
            "Claude Code: No conversation found with session ID: x"
        );
    }

    #[test]
    fn arguments_load_only_agent_desktop_and_allow_it_without_prompts() {
        let mut run = Run {
            backend: AssistantBackend::ClaudeCode,
            model: Some("sonnet".into()),
            system: "You are Hex.".into(),
            prompt: "hi".into(),
            image: None,
            resume: Some("abc".into()),
            history: Vec::new(),
            desktop: None,
            workdir: std::env::temp_dir(),
            quick: true,
        };
        let plain = arguments(&run);
        assert!(!plain.iter().any(|arg| arg.contains("mcpServers")));
        assert!(plain.windows(2).any(|pair| pair == ["--resume", "abc"]));
        assert!(
            plain
                .windows(2)
                .any(|pair| pair == ["--allowedTools", "WebSearch,WebFetch"])
        );
        run.desktop = Some(Desktop::fixture(Some("s1"), "hex-2"));
        let args = arguments(&run);
        let config = &args[args.iter().position(|arg| arg == "--mcp-config").unwrap() + 1];
        let config: Value = serde_json::from_str(config).unwrap();
        assert_eq!(
            config.pointer("/mcpServers/agent-desktop/args").unwrap(),
            &json!(["mcp", "--session", "s1", "--agent-id", "hex-2"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--allowedTools", "WebSearch,WebFetch,mcp__agent-desktop"])
        );
        let withheld = &args[args
            .iter()
            .position(|arg| arg == "--disallowedTools")
            .unwrap()
            + 1];
        assert!(withheld.contains("mcp__agent-desktop__desktop_session_end"));
    }
}
