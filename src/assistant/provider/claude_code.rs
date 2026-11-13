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

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let input = input(run)?;
    let key = warm_key(run);
    if let Some(process) = warm::take(&key, run.resume.as_deref()) {
        tracing::debug!("the assistant uses a Claude Code that is already running");
        control.track(&process.tree);
        if let Some(result) = turn(process, &input, run, &key, control, report, true) {
            return result;
        }
        tracing::info!("the waiting Claude Code had ended; starting a new one");
    }
    let process = spawn(run, control)?;
    turn(process, &input, run, &key, control, report, false)
        .unwrap_or_else(|| Err(eyre!("Claude Code stopped before it answered")))
}

/// Starts Claude Code for `run` and leaves it waiting for its question, at
/// most `ttl`.
pub(super) fn prewarm(run: &Run, ttl: Duration) {
    let key = warm_key(run);
    if warm::has(&key, run.resume.as_deref()) {
        return;
    }
    match spawn(run, &Control::default()) {
        Ok(process) => warm::put(key, run.resume.clone(), process, ttl),
        Err(error) => tracing::warn!(%error, "couldn't start Claude Code ahead"),
    }
}

fn spawn(run: &Run, control: &Control) -> Result<Process> {
    let executable = super::executable(AssistantBackend::ClaudeCode)?;
    let mut command = super::command(&executable, &run.workdir);
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
    format!("{args:?}{:?}", run.workdir)
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
    let mut escalate = None;
    if wrote {
        while let Some(line) = process.next_line() {
            lines += 1;
            report(Report::Alive);
            stream.line(&line, report);
            if stream.outcome.is_some() {
                break;
            }
            if stream.hands_over(run.escalate_after) {
                escalate = Some(super::Escalate {
                    tools: stream.answered,
                });
                break;
            }
        }
    }
    if reused && lines == 0 && !control.cancelled() {
        let _ = process.finish(control);
        return None;
    }
    // Between two steps, with no tool running: the session on disk ends
    // with the last tool's result, and the strong model goes on from there.
    if let Some(escalate) = escalate {
        tracing::info!(
            tools = escalate.tools,
            "the fast model hands the turn to the strong one"
        );
        super::end_group(process.tree.clone());
        let _ = process.finish(control);
        return Some(Err(escalate.into()));
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

/// Built-in tools a job gets beside web search and fetch: files and a shell
/// in its workspace, and subagents for parallel batches.
const JOB_TOOLS: &[&str] = &["Read", "Write", "Edit", "Glob", "Grep", "Bash", "Task"];

/// Whether Claude Code can sandbox a job's shell here: Seatbelt on macOS,
/// bubblewrap (`bwrap`) and `socat` on Linux; it has no sandbox on Windows.
/// The error says what's missing, for the self-test.
pub(crate) fn sandbox() -> std::result::Result<(), String> {
    if cfg!(target_os = "macos") {
        return Ok(());
    }
    if cfg!(windows) {
        return Err("Claude Code has no sandbox on Windows".into());
    }
    let find = crate::assistant::backends::search::find;
    let missing: Vec<&str> = ["bwrap", "socat"]
        .into_iter()
        .filter(|program| find(program).is_none())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Claude Code's sandbox needs bubblewrap and socat; {} {} missing \
             (install the bubblewrap and socat packages)",
            missing.join(" and "),
            if missing.len() == 1 { "is" } else { "are" }
        ))
    }
}

/// `--settings`: hooks off; for a job, its file tools allowed only inside
/// the workspace, and (when `sandboxed`) its shell sandboxed (writes only
/// in the workspace, which is the working folder; the network only to
/// install Python packages) and run without asking. Without a sandbox the
/// job has no shell at all ([`arguments_with`]).
fn settings(run: &Run, sandboxed: bool) -> String {
    let Some(workspace) = &run.workspace else {
        return r#"{"disableAllHooks":true}"#.into();
    };
    // Yolo: the person lifted every limit. No sandbox, every path, the
    // whole network, no prompts (`--permission-mode bypassPermissions`).
    if run.unrestricted {
        return json!({
            "disableAllHooks": true,
            "sandbox": {"enabled": false},
            "permissions": {"defaultMode": "bypassPermissions"},
        })
        .to_string();
    }
    // `//` starts an absolute path in a permission rule.
    let scope = format!("/{}/**", rule_path(&workspace.to_string_lossy()));
    let allow: Vec<String> = ["Read", "Write", "Edit", "Glob", "Grep"]
        .iter()
        .map(|tool| format!("{tool}({scope})"))
        .collect();
    let mut settings = json!({
        "disableAllHooks": true,
        "permissions": {"allow": allow},
    });
    settings["sandbox"] = if sandboxed {
        json!({
            "enabled": true,
            "autoAllowBashIfSandboxed": true,
            "allowUnsandboxedCommands": false,
            "network": {"allowedDomains": ["pypi.org", "files.pythonhosted.org"]},
        })
    } else {
        json!({"enabled": false})
    };
    if !sandboxed {
        settings["permissions"]["deny"] = json!(["Bash"]);
    }
    settings.to_string()
}

/// A path as Claude Code matches it in a permission rule: as it is on
/// Unix; on Windows in POSIX form, `C:\Users\p` → `/c/Users/p`.
fn rule_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if !drive {
        return path.replace('\\', "/");
    }
    let rest = path[2..].replace('\\', "/");
    format!(
        "/{}/{}",
        path[..1].to_ascii_lowercase(),
        rest.trim_start_matches('/')
    )
}

fn arguments(run: &Run) -> Vec<String> {
    let sandboxed = run.workspace.is_some() && !run.unrestricted && sandbox().is_ok();
    arguments_with(run, sandboxed)
}

/// The arguments for `run`; without a sandbox a job gets no shell (its file
/// tools stay scoped to the workspace).
fn arguments_with(run: &Run, sandboxed: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        if run.unrestricted {
            "bypassPermissions"
        } else {
            "dontAsk"
        },
        "--strict-mcp-config",
        // Only the person's own settings and Agent-Desk's (`--settings`): never
        // a project's `.claude/settings*.json`, which a job could write into
        // its own workspace (its working folder) to widen its sandbox's
        // network or run commands of its own.
        "--setting-sources",
        "user",
    ]
    .map(str::to_owned)
    .into();
    args.extend(["--settings".into(), settings(run, sandboxed)]);
    let mut tools = vec!["WebSearch".to_owned(), "WebFetch".to_owned()];
    let mut allowed = tools.clone();
    if let Some(workspace) = &run.workspace {
        let shell = sandboxed || run.unrestricted;
        tools.extend(
            JOB_TOOLS
                .iter()
                .filter(|tool| shell || **tool != "Bash")
                .map(|tool| (*tool).to_owned()),
        );
        if shell {
            allowed.push("Bash".to_owned());
        }
        if run.unrestricted {
            allowed.extend(["Read", "Write", "Edit", "Glob", "Grep"].map(str::to_owned));
        }
        allowed.push("Task".to_owned());
        args.extend(["--add-dir".into(), workspace.display().to_string()]);
    }
    args.extend(["--tools".into(), tools.join(",")]);
    args.extend(["--system-prompt".into(), run.system.clone()]);
    if let Some(config) = super::servers::claude_config(run) {
        args.extend(["--mcp-config".into(), config]);
        allowed.extend(super::servers::claude_tools(run));
    }
    args.extend(["--allowedTools".into(), allowed.join(",")]);
    if let Some(model) = run
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
    {
        args.extend(["--model".into(), model.trim().to_owned()]);
    }
    if let Some(effort) = run
        .effort
        .as_deref()
        .filter(|effort| !effort.trim().is_empty())
    {
        args.extend(["--effort".into(), effort.trim().to_owned()]);
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
    /// Tool calls made and not yet answered.
    running: std::collections::HashSet<String>,
    /// Tool calls answered.
    answered: usize,
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
                        self.running.insert(id.to_owned());
                    }
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    report(Report::tool(name, &input));
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
                    let id = block.get("tool_use_id").and_then(Value::as_str);
                    if let Some(id) = id
                        && self.running.remove(id)
                    {
                        self.answered += 1;
                    }
                    let Some(name) = id.and_then(|id| self.tools.get(id)).cloned() else {
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

    /// Whether a fast model's turn stops here for the strong one: `after`
    /// tool calls have answered and none is running, so nothing is cut off
    /// mid-action.
    pub(super) fn hands_over(&self, after: Option<usize>) -> bool {
        after.is_some_and(|after| self.answered >= after && self.running.is_empty())
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
    fn arguments_load_only_the_desktop_tools_and_allow_them_without_prompts() {
        let mut run = Run {
            model: Some("sonnet".into()),
            resume: Some("abc".into()),
            quick: true,
            ..Run::fixture(AssistantBackend::ClaudeCode)
        };
        let plain = arguments(&run);
        assert!(!plain.iter().any(|arg| arg.contains("mcpServers")));
        assert!(
            plain
                .windows(2)
                .any(|pair| pair == ["--setting-sources", "user"]),
            "never a project's settings, which a job could write"
        );
        assert!(plain.windows(2).any(|pair| pair == ["--resume", "abc"]));
        assert!(
            plain
                .windows(2)
                .any(|pair| pair == ["--allowedTools", "WebSearch,WebFetch"])
        );
        run.desktop = Some(Desktop::fixture());
        let args = arguments(&run);
        let config = &args[args.iter().position(|arg| arg == "--mcp-config").unwrap() + 1];
        let config: Value = serde_json::from_str(config).unwrap();
        assert_eq!(
            config.pointer("/mcpServers/desktop/args").unwrap(),
            &json!([
                "assistant",
                "mcp",
                "--spec",
                r#"{"task":"t","lane":"background"}"#
            ])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--allowedTools", "WebSearch,WebFetch,mcp__desktop"])
        );
        assert!(!args.iter().any(|arg| arg == "--disallowedTools"));
    }

    #[test]
    fn a_job_gets_files_a_sandboxed_shell_and_subagents_in_its_workspace() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        let plain = arguments(&run);
        assert!(
            plain
                .windows(2)
                .any(|pair| pair == ["--tools", "WebSearch,WebFetch"])
        );
        assert!(!plain.iter().any(|arg| arg.contains("sandbox")));
        run.workspace = Some("/Users/p/Documents/Agent-Desk/2026-11-12 Job".into());
        let args = arguments_with(&run, true);
        assert!(args.windows(2).any(|pair| pair
            == [
                "--tools",
                "WebSearch,WebFetch,Read,Write,Edit,Glob,Grep,Bash,Task"
            ]));
        let allowed = &args[args.iter().position(|arg| arg == "--allowedTools").unwrap() + 1];
        assert_eq!(allowed, "WebSearch,WebFetch,Bash,Task");
        // The built-ins the rules name are the ones Claude Code gets.
        let tools = |args: &[String]| {
            args[args.iter().position(|arg| arg == "--tools").unwrap() + 1].clone()
        };
        for (args, job) in [(&plain, false), (&args, true)] {
            let tools = tools(args);
            for name in
                crate::assistant::agent::toolset::Builtins::of(AssistantBackend::ClaudeCode, job)
                    .names()
            {
                assert!(tools.split(',').any(|tool| tool == name), "{name}");
            }
        }
        let settings: Value = serde_json::from_str(
            &args[args.iter().position(|arg| arg == "--settings").unwrap() + 1],
        )
        .unwrap();
        assert!(
            settings["permissions"]["allow"]
                .as_array()
                .unwrap()
                .contains(&json!(
                    "Write(//Users/p/Documents/Agent-Desk/2026-11-12 Job/**)"
                ))
        );
        assert_eq!(settings["sandbox"]["enabled"], true);
        assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    }

    #[test]
    fn without_a_sandbox_a_job_has_no_shell() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        run.workspace = Some("/home/p/Documents/Agent-Desk/2026-11-12 Job".into());
        let args = arguments_with(&run, false);
        assert!(args.windows(2).any(|pair| pair
            == [
                "--tools",
                "WebSearch,WebFetch,Read,Write,Edit,Glob,Grep,Task"
            ]));
        let allowed = &args[args.iter().position(|arg| arg == "--allowedTools").unwrap() + 1];
        assert_eq!(allowed, "WebSearch,WebFetch,Task");
        let settings: Value = serde_json::from_str(
            &args[args.iter().position(|arg| arg == "--settings").unwrap() + 1],
        )
        .unwrap();
        assert_eq!(settings["sandbox"]["enabled"], false);
        assert_eq!(settings["permissions"]["deny"], json!(["Bash"]));
    }

    #[test]
    fn a_yolo_job_runs_without_sandbox_or_prompts() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        run.workspace = Some("/Users/p/Documents/Agent-Desk/2026-11-12 Shop".into());
        run.unrestricted = true;
        // Even where a sandbox exists, Yolo doesn't use it.
        for args in [arguments(&run), arguments_with(&run, false)] {
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--permission-mode", "bypassPermissions"])
            );
            assert!(args.windows(2).any(|pair| pair
                == [
                    "--tools",
                    "WebSearch,WebFetch,Read,Write,Edit,Glob,Grep,Bash,Task"
                ]));
            let allowed = &args[args.iter().position(|arg| arg == "--allowedTools").unwrap() + 1];
            assert!(
                allowed.contains("Bash") && allowed.contains("Write"),
                "{allowed}"
            );
            let settings: Value = serde_json::from_str(
                &args[args.iter().position(|arg| arg == "--settings").unwrap() + 1],
            )
            .unwrap();
            assert_eq!(settings["sandbox"]["enabled"], false);
            assert!(settings["permissions"]["deny"].is_null());
            assert!(
                settings["permissions"]["allow"].is_null(),
                "no workspace-only scope"
            );
            assert!(!settings.to_string().contains("allowedDomains"));
        }
        // Outside Yolo a job keeps its sandbox and asks nothing it can't.
        run.unrestricted = false;
        let args = arguments_with(&run, true);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--permission-mode", "dontAsk"])
        );
    }

    #[test]
    fn windows_paths_become_posix_in_permission_rules() {
        assert_eq!(
            rule_path(r"C:\Users\p\Documents\Agent-Desk\2026-11-12 Job"),
            "/c/Users/p/Documents/Agent-Desk/2026-11-12 Job"
        );
        assert_eq!(rule_path(r"\\?\D:\Jobs"), "/d/Jobs");
        assert_eq!(rule_path("/home/p/Job"), "/home/p/Job");
    }

    #[test]
    fn a_waiting_process_fits_only_the_same_run() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        run.desktop = Some(Desktop::fixture());
        let key = warm_key(&run);
        // The session it resumes is matched apart from the arguments.
        run.resume = Some("abc".into());
        assert_eq!(warm_key(&run), key);
        // Another task's guard: another process.
        let mut other = Desktop::fixture();
        other.server.args[3] = r#"{"task":"u","lane":"background"}"#.into();
        run.desktop = Some(other);
        assert_ne!(warm_key(&run), key);
    }

    #[test]
    fn a_fast_turn_hands_over_only_between_steps() {
        let fixture = include_str!("../../../tests/fixtures/assistant/claude-code-click.jsonl");
        let mut stream = Stream::default();
        let mut handed = Vec::new();
        for line in fixture.lines() {
            stream.line(line, &mut |_| {});
            handed.push((stream.answered, stream.hands_over(Some(2))));
        }
        // After the first tool's result: one answered, not yet.
        assert!(handed.contains(&(1, false)));
        // Before the second tool's result it never hands over.
        assert!(!handed.iter().any(|(answered, over)| *answered < 2 && *over));
        // Answered twice with nothing running: it hands over.
        assert!(handed.contains(&(2, true)));
        assert!(!stream.hands_over(None), "never without a limit");
    }

    #[test]
    fn a_quick_turn_names_its_effort() {
        let mut run = Run {
            model: Some("claude-sonnet-5-5".into()),
            effort: Some("low".into()),
            quick: true,
            ..Run::fixture(AssistantBackend::ClaudeCode)
        };
        let args = arguments(&run);
        assert!(args.windows(2).any(|pair| pair == ["--effort", "low"]));
        run.effort = Some(" ".into());
        assert!(!arguments(&run).iter().any(|arg| arg == "--effort"));
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
