//! Claude Code: `claude -p` with stream-json in and out, signed in with the
//! person's own account.
//!
//! The question (and a quick question's screenshot, as an image block) goes
//! in on stdin as one stream-json user message. Only the task's MCP servers
//! are loaded (`--strict-mcp-config`: its desktop tool and its browser), the
//! built-in tools are cut to web search and fetch, and `dontAsk` denies
//! anything not allowed up front, so a task never waits on a permission
//! prompt. Hooks are off: the person's own hooks are for their coding
//! sessions, not this.
//!
//! Its input stays open: a process started ahead (`prewarm`) waits for the
//! question, and after an answer a Here conversation's process can wait
//! (`Run::keep_warm`) for the follow-up, which then skips start-up entirely.

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin, warm};
use crate::app_settings::AssistantBackend;

/// How long a process started ahead waits for its question.
const PREWARM_TTL: Duration = Duration::from_secs(90);

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let input = input(run)?;
    let key = warm_key(run);
    if let Some(process) = warm::take(&key, run.resume.as_deref()) {
        tracing::debug!("the assistant uses a Claude Code that is already running");
        control.track(process.child.id());
        if let Some(result) = turn(process, &input, run, &key, control, report, true) {
            return result;
        }
        tracing::info!("the waiting Claude Code had ended; starting a new one");
    }
    let process = spawn(run, control)?;
    turn(process, &input, run, &key, control, report, false)
        .unwrap_or_else(|| Err(eyre!("Claude Code stopped before it answered")))
}

/// Starts Claude Code for `run` and leaves it waiting for its question.
pub(super) fn prewarm(run: &Run) {
    let key = warm_key(run);
    if warm::has(&key, run.resume.as_deref()) {
        return;
    }
    match spawn(run, &Control::default()) {
        Ok(process) => warm::put(key, run.resume.clone(), process, PREWARM_TTL),
        Err(error) => tracing::warn!(%error, "couldn't start Claude Code ahead"),
    }
}

fn spawn(run: &Run, control: &Control) -> Result<Process> {
    let executable = super::executable(AssistantBackend::ClaudeCode)?;
    let mut command = super::command(&executable, &run.workdir, run.desktop.as_ref());
    command.args(arguments(run));
    Process::spawn(command, Stdin::Keep, control)
}

/// What makes a waiting process fit `run`: everything it was started with
/// but the session it resumed, which is matched on its own.
fn warm_key(run: &Run) -> String {
    let mut args = arguments(run);
    if let Some(index) = args.iter().position(|arg| arg == "--resume") {
        args.drain(index..(index + 2).min(args.len()));
    }
    let env = run.desktop.as_ref().map(super::Desktop::env);
    format!("{args:?}{:?}{env:?}", run.workdir)
}

/// Sends one question to `process` and reads until its result. `None` when
/// a waiting process (`reused`) turned out to have ended before answering.
/// Keeps the process for the follow-up when `run` asks to and it answered.
fn turn(
    mut process: Process,
    input: &[u8],
    run: &Run,
    key: &str,
    control: &Control,
    report: &mut dyn FnMut(Report),
    reused: bool,
) -> Option<Result<Finished>> {
    let wrote = process
        .stdin()
        .is_some_and(|pipe| pipe.write_all(input).and_then(|()| pipe.flush()).is_ok());
    let mut stream = Stream::default();
    let mut lines = 0;
    if wrote {
        while let Some(line) = process.next_line() {
            lines += 1;
            stream.line(&line, report);
            if stream.outcome.is_some() {
                break;
            }
        }
    }
    if reused && lines == 0 && !control.cancelled() {
        let _ = process.finish(control);
        return None;
    }
    let answered = matches!(stream.outcome, Some(Ok(_)));
    if answered
        && !control.cancelled()
        && let Some(ttl) = run.keep_warm
    {
        let pid = process.child.id();
        control.untrack(pid);
        let session = stream.session.clone().or_else(|| run.resume.clone());
        warm::put(key.to_owned(), session, process, ttl);
        return stream.finish();
    }
    drop(process.stdin.take());
    let (status, stderr) = process.finish(control);
    if control.cancelled() {
        return Some(Err(eyre!("cancelled")));
    }
    Some(stream.finish().unwrap_or_else(|| {
        Err(super::exit_error(
            AssistantBackend::ClaudeCode,
            status,
            &stderr,
        ))
    }))
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
    if let Some(config) = super::servers::claude_config(run) {
        args.extend(["--mcp-config".into(), config]);
        let (servers, withheld) = super::servers::claude_tools(run);
        allowed.extend(servers);
        if !withheld.is_empty() {
            args.extend(["--disallowedTools".into(), withheld.join(",")]);
        }
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

/// A recorded limit's words when the result says nothing more.
fn limit_text(event: &Value) -> Option<String> {
    (event.get("error").and_then(Value::as_str) == Some("rate_limit")).then(|| {
        blocks(event)
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    })
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
    /// The backend session this turn ran in.
    session: Option<String>,
    /// A synthetic message that reported a usage limit.
    limit: Option<String>,
}

impl Stream {
    pub(super) fn line(&mut self, line: &str, report: &mut dyn FnMut(Report)) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("system") if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                if let Some(session) = event.get("session_id").and_then(Value::as_str) {
                    self.session = Some(session.to_owned());
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
                if let Some(limit) = limit_text(&event) {
                    self.limit = Some(limit);
                }
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
                        self.limit
                            .clone()
                            .unwrap_or_else(|| "it stopped without an answer".into())
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
            model: Some("sonnet".into()),
            resume: Some("abc".into()),
            quick: true,
            ..Run::fixture(AssistantBackend::ClaudeCode)
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

    #[test]
    fn a_waiting_process_fits_only_the_same_run() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        run.desktop = Some(Desktop::fixture(Some("s1"), "hex-2"));
        let key = warm_key(&run);
        // The session it resumes is matched apart from the arguments.
        run.resume = Some("abc".into());
        assert_eq!(warm_key(&run), key);
        run.desktop = Some(Desktop::fixture(Some("s1"), "hex-3"));
        assert_ne!(warm_key(&run), key);
    }

    #[test]
    fn a_recorded_usage_limit_fails_with_its_reset_time() {
        let (_, outcome) = replay(include_str!(
            "../../../tests/fixtures/assistant/claude-code-limit.jsonl"
        ));
        let error = outcome.unwrap().unwrap_err().to_string();
        assert_eq!(
            error,
            "Claude Code: You've hit your session limit · resets 2am (Europe/Berlin)"
        );
        assert!(
            crate::assistant::provider::limit::detect(&error)
                .unwrap()
                .until_ms
                .is_some()
        );
    }
}
