//! OpenCode: `opencode run --format json` (OpenCode 2 with `--standalone`),
//! with the person's own OpenCode configuration and sign-ins. Classic
//! OpenCode (1.x) and OpenCode 2 take different flags and config words
//! ([`backends::opencode`](crate::assistant::backends::opencode)).
//!
//! Why the CLI and not the running service: Agent-Desk's other features only talk
//! to an already running OpenCode service, but that service loads its MCP
//! servers once, from its own configuration, and its API has no way to give
//! one session its own desktop tool (this task's guard).
//! `--standalone` runs a private server for just this task, and
//! `OPENCODE_CONFIG_CONTENT` adds to the person's configuration an agent
//! `agent-desk` (Agent-Desk's instructions; shell and file edits denied) and the
//! guard as a local MCP server. `--auto` approves the rest, so nothing waits on a
//! prompt; `--archived` keeps these sessions out of the person's list
//! (OpenCode 2; Classic OpenCode titles them "Agent-Desk" instead).
//! A conversation continues with `--session <id>`.

use color_eyre::{Result, eyre::eyre};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin};
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::opencode::{self as cli, Generation};

const AGENT: &str = "hex";

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let executable = super::executable(AssistantBackend::OpenCode)?;
    let generation = cli::generation_for(&executable);
    let mut command = super::command(&executable, &run.workdir);
    // A bring-your-own-key provider's key travels in the environment only;
    // the config names it by reference.
    if let Some(env) = &run.backend_env {
        command
            .env_clear()
            .envs(env.iter().map(|(key, value)| (key, value)));
    }
    command.env(
        "OPENCODE_CONFIG_CONTENT",
        config(run, generation).to_string(),
    );
    // A Windows batch file can't take the prompt's newlines and quotes as
    // an argument; when OpenCode is one Agent-Desk couldn't unwrap, the prompt goes
    // on its input, which `opencode run` reads as the message.
    let batch = is_batch_file(&crate::assistant::backends::search::launch(&executable).0);
    let mut args = arguments(run, generation);
    let stdin = if batch {
        args.truncate(args.len() - 2);
        Stdin::Write(run.prompt.clone().into_bytes())
    } else {
        Stdin::Null
    };
    command.args(args);
    let mut process = Process::spawn(command, stdin, control)?;
    let mut stream = Stream::default();
    while let Some(line) = process.next_line() {
        stream.line(&line, report);
        // A quick turn on the fast model that needs many tools: the strong
        // model goes on in this session. Only between two steps, once the
        // step's every tool (several may run at once) has its result, so
        // the session it resumes has no call left open.
        if stream.between_steps
            && run
                .escalate_after
                .is_some_and(|after| stream.error.is_none() && stream.tools >= after)
        {
            tracing::info!(
                tools = stream.tools,
                "the fast model hands the turn to the strong one"
            );
            super::end_group(process.tree.clone());
            let _ = process.finish(control);
            return Err(super::Escalate {
                tools: stream.tools,
            }
            .into());
        }
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

fn arguments(run: &Run, generation: Generation) -> Vec<String> {
    let mut args: Vec<String> = cli::run_args(generation)
        .into_iter()
        .chain(["--agent", AGENT])
        .map(str::to_owned)
        .collect();
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

/// A `.cmd` or `.bat` file (Windows).
fn is_batch_file(program: &std::path::Path) -> bool {
    cfg!(windows)
        && program.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

/// What `OPENCODE_CONFIG_CONTENT` adds to the person's configuration, in
/// `generation`'s words.
fn config(run: &Run, generation: Generation) -> Value {
    // A job may edit and run commands in its workspace (the working
    // folder); nothing outside it.
    let permission = match run.workspace {
        // Yolo: everything, anywhere.
        Some(_) if run.unrestricted => json!({"bash": "allow", "edit": "allow",
            "webfetch": "allow", "external_directory": "allow"}),
        Some(_) => json!({"bash": "allow", "edit": "allow", "webfetch": "allow",
            "external_directory": "deny"}),
        None => json!({"bash": "deny", "edit": "deny", "webfetch": "allow"}),
    };
    let mut config = json!({"agent": {AGENT: {
        "mode": "primary",
        "description": "Agent-Desk's assistant",
        "prompt": run.system,
        "permission": permission,
    }}});
    if let Some(mcp) = super::servers::opencode_config(run) {
        config["mcp"] = mcp;
    }
    if let Some(provider) = &run.provider_config {
        crate::assistant::backends::merge_config(&mut config, provider);
    }
    cli::adapt_config(&mut config, generation);
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
    /// The tool calls made so far.
    tools: usize,
    /// The last event ended a step (`step_finish`): no tool runs now.
    between_steps: bool,
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
        // Any event (a step starting too) says OpenCode still works.
        report(Report::Alive);
        let part = event.get("part").unwrap_or(&Value::Null);
        let kind = event.get("type").and_then(Value::as_str);
        self.between_steps = kind == Some("step_finish");
        match kind {
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
                self.tools += 1;
                self.answer.clear();
                if self.wrote {
                    self.tool_after_text = true;
                }
                for (tool, args) in calls(part) {
                    report(Report::tool(&tool, &args));
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
            // Between two steps: the session's size (what its last request
            // carried), so a long job starts a fresh session in time
            // (`agent::rollover`), as on Claude Code.
            Some("step_finish") => {
                let tokens = |path: &str| part.pointer(path).and_then(Value::as_u64).unwrap_or(0);
                let context_tokens = tokens("/tokens/input")
                    + tokens("/tokens/cache/read")
                    + tokens("/tokens/cache/write");
                if context_tokens > 0 {
                    report(Report::Usage(crate::assistant::agent::rollover::Usage {
                        context_tokens,
                    }));
                }
                if let Some(cost) = part
                    .get("cost")
                    .and_then(Value::as_f64)
                    .filter(|cost| cost.is_finite() && *cost > 0.0)
                {
                    report(Report::Cost(cost));
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
                // Keep what tells a limit apart: the HTTP status, and when
                // to try again.
                let data = event.pointer("/error/data");
                let status = data
                    .and_then(|data| data.get("statusCode"))
                    .and_then(Value::as_u64);
                let retry = data
                    .and_then(|data| data.pointer("/responseHeaders/retry-after"))
                    .and_then(Value::as_str);
                let mut why = why;
                if let Some(status) = status {
                    why.push_str(&format!(" (HTTP {status})"));
                }
                if let Some(seconds) = retry.and_then(|retry| retry.trim().parse::<u64>().ok()) {
                    why.push_str(&format!("; retry after {seconds} seconds"));
                }
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
/// mode (`execute`), each `tools["desktop"].desktop_*` call in its code.
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
                    let name = tool
                        .strip_prefix(crate::assistant::backends::MCP_NAME)?
                        .strip_prefix('.')?;
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
    fn a_yolo_job_may_work_outside_its_workspace() {
        let mut run = Run {
            workspace: Some("/tmp/job".into()),
            ..Run::fixture(AssistantBackend::OpenCode)
        };
        let permission =
            |run: &Run| config(run, Generation::Two)["agent"][AGENT]["permission"].clone();
        assert_eq!(permission(&run)["external_directory"], "deny");
        run.unrestricted = true;
        assert_eq!(permission(&run)["external_directory"], "allow");
        assert_eq!(permission(&run)["bash"], "allow");
    }

    #[test]
    fn code_mode_calls_are_read_from_their_code() {
        let calls = code_calls(
            r#"const s = await tools["desktop"].desktop_snapshot({ app: 'Mail', interactive_only: true });
               await tools["desktop"].desktop_click({"ref_id": "@s1:e2"});"#,
        );
        assert_eq!(calls[0].0, "desktop_snapshot");
        assert_eq!(calls[0].1, json!({"app": "Mail", "interactive_only": true}));
        assert_eq!(
            calls[1],
            ("desktop_click".into(), json!({"ref_id": "@s1:e2"}))
        );
    }

    #[test]
    fn a_bring_your_own_key_provider_is_merged_into_the_config() {
        let mut run = Run::fixture(AssistantBackend::OpenCode);
        run.provider_config = Some(json!({
            "model": "deepseek/deepseek-v4-pro",
            "providers": {"deepseek": {"settings": {"apiKey": "{env:DEEPSEEK_API_KEY}"}}},
        }));
        let two = config(&run, Generation::Two);
        assert_eq!(two["model"], "deepseek/deepseek-v4-pro");
        assert_eq!(
            two["providers"]["deepseek"]["settings"]["apiKey"],
            "{env:DEEPSEEK_API_KEY}"
        );
        // Agent-Desk's own agent stays.
        assert_eq!(two["agent"]["hex"]["mode"], "primary");
        // Classic OpenCode rejects `providers`: its own words instead.
        let classic = config(&run, Generation::Classic);
        assert!(classic.get("providers").is_none());
        assert_eq!(
            classic["provider"]["deepseek"]["options"]["apiKey"],
            "{env:DEEPSEEK_API_KEY}"
        );
        assert_eq!(classic["agent"]["hex"]["mode"], "primary");
    }

    #[test]
    fn each_opencode_gets_only_the_flags_it_knows() {
        let mut run = Run::fixture(AssistantBackend::OpenCode);
        run.resume = Some("ses_1".into());
        run.model = Some("deepseek/deepseek-v4-pro".into());
        let classic = arguments(&run, Generation::Classic);
        assert!(
            !classic
                .iter()
                .any(|arg| arg == "--standalone" || arg == "--archived")
        );
        let two = arguments(&run, Generation::Two);
        assert!(two.iter().any(|arg| arg == "--standalone"));
        for args in [classic, two] {
            assert_eq!(args[0], "run");
            let after = |flag: &str| {
                args.iter()
                    .position(|arg| arg == flag)
                    .map(|at| args[at + 1].clone())
            };
            assert_eq!(after("--agent").as_deref(), Some(AGENT));
            assert_eq!(after("--session").as_deref(), Some("ses_1"));
            assert_eq!(
                after("--model").as_deref(),
                Some("deepseek/deepseek-v4-pro")
            );
            assert_eq!(args[args.len() - 2..], ["--".to_owned(), "hi".to_owned()]);
        }
    }

    #[test]
    fn each_step_reports_the_sessions_size() {
        let mut stream = Stream::default();
        let mut sizes = Vec::new();
        for line in include_str!("../../../tests/fixtures/assistant/opencode-desktop.jsonl").lines()
        {
            stream.line(line, &mut |report| {
                if let Report::Usage(usage) = report {
                    sizes.push(usage.context_tokens);
                }
            });
        }
        // input + cache read + cache write of each step.
        assert_eq!(sizes[..3], [10_768, 11_853, 15_991]);
        // Classic OpenCode's shape (a `total` beside them) counts the same.
        let mut classic = Vec::new();
        Stream::default().line(
            r#"{"type":"step_finish","sessionID":"s","part":{"type":"step-finish","tokens":{"total":11139,"input":9473,"output":2,"reasoning":0,"cache":{"write":0,"read":1664}},"cost":0.0014}}"#,
            &mut |report| {
                if let Report::Usage(usage) = report {
                    classic.push(usage.context_tokens);
                }
            },
        );
        assert_eq!(classic, [11_137]);
        let mut cost = 0.0;
        Stream::default().line(
            r#"{"type":"step_finish","sessionID":"s","part":{"type":"step-finish","tokens":{"input":9,"cache":{}},"cost":0.0014}}"#,
            &mut |report| {
                if let Report::Cost(usd) = report {
                    cost += usd;
                }
            },
        );
        assert!((cost - 0.0014).abs() < 1e-9);
    }

    #[test]
    fn tool_calls_are_counted_for_the_hand_over_between_steps() {
        let mut stream = Stream::default();
        let mut boundaries = Vec::new();
        for line in include_str!("../../../tests/fixtures/assistant/opencode-desktop.jsonl").lines()
        {
            stream.line(line, &mut |_| {});
            if stream.between_steps {
                boundaries.push(stream.tools);
            }
        }
        assert!(stream.tools > 0);
        // A hand-over waits for the step's end, never right after a tool.
        assert!(!boundaries.is_empty());
        stream.line(
            r#"{"type":"tool_use","sessionID":"s","part":{"tool":"x","state":{"status":"completed","input":{}}}}"#,
            &mut |_| {},
        );
        assert!(!stream.between_steps);
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

    #[test]
    fn a_rate_limit_keeps_its_status_and_wait() {
        let mut stream = Stream::default();
        stream.line(
            r#"{"type":"error","sessionID":"ses_1","error":{"name":"APIError","data":{"message":"Rate limit exceeded","statusCode":429,"isRetryable":true,"responseHeaders":{"retry-after":"42"}}}}"#,
            &mut |_| {},
        );
        let error = stream.finish().unwrap().unwrap_err().to_string();
        assert_eq!(
            error,
            "OpenCode: Rate limit exceeded (HTTP 429); retry after 42 seconds"
        );
        let limit = crate::assistant::provider::limit::detect_at(&error, 1_000, 0).unwrap();
        assert_eq!(limit.until_ms, Some(43_000));
    }
}
