//! The agents' own self-tests: each installed agent, and the Claude API,
//! is asked to reply "OK", and its events are read for the answer, an
//! error, or a signed-out account.

use super::*;

pub(super) const ANSWER_LIMIT: Duration = Duration::from_secs(30);
pub(super) const MCP_LIMIT: Duration = Duration::from_secs(60);
pub(super) const ANSWER_PROMPT: &str = "Reply with exactly: OK";

/// The installed agent, or the check failing with its install fix.
pub(super) fn installed(check: Check, backend: AssistantBackend) -> Result<PathBuf, CheckResult> {
    super::super::executable(backend).ok_or_else(|| {
        CheckResult::failed(
            check,
            format!("{} isn't installed", name(backend)),
            Some(Fix::Install(backend)),
        )
    })
}

pub(super) fn answers(
    check: Check,
    backend: AssistantBackend,
    settings: &AssistantSettings,
) -> CheckResult {
    if backend == AssistantBackend::ClaudeApi {
        return api_answers(check, settings);
    }
    let path = match installed(check, backend) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let mut command = agent_command(&path);
    command.current_dir(scratch());
    match backend {
        AssistantBackend::ClaudeCode => {
            command.args([
                "-p",
                ANSWER_PROMPT,
                "--output-format",
                "stream-json",
                "--verbose",
            ]);
            command.args([
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
            ]);
            if let Some(model) = &settings.claude_code_model {
                command.args(["--model", model]);
            }
        }
        AssistantBackend::Codex => {
            command.args(codex_base(&scratch()));
            if let Some(model) = &settings.codex_model {
                command.args(["--model", model]);
            }
            command.arg(ANSWER_PROMPT);
        }
        AssistantBackend::OpenCode => {
            // Standalone, as tasks run it, with the person's own key when
            // one is chosen.
            let model = super::super::providers::opencode_check(
                &mut command,
                settings,
                serde_json::json!({}),
            );
            command.args(["run", "--standalone", "--format", "json", "--archived"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(ANSWER_PROMPT);
        }
        AssistantBackend::ClaudeApi => unreachable!("answered above"),
    }
    let ran = match run::bounded(command, ANSWER_LIMIT) {
        Ok(ran) => ran,
        Err(error) => {
            return CheckResult::failed(
                check,
                format!("{} didn't start: {error}", name(backend)),
                Some(Fix::Install(backend)),
            );
        }
    };
    let events = parse_events(&ran.stdout);
    let text = answer_text(&events);
    if text.to_ascii_uppercase().contains("OK") {
        let mut detail = format!(
            "Replied “{}” in {:.1} s",
            run::shorten(text.trim(), 40),
            ran.took.as_secs_f32()
        );
        // Without its sandbox a job gets no shell; say so here.
        if backend == AssistantBackend::ClaudeCode
            && let Err(why) = crate::assistant::provider::claude_code_sandbox()
        {
            detail.push_str(&format!(". Jobs run without a shell: {why}"));
        }
        return CheckResult::passed(check, detail);
    }
    let problem = event_error(&events).unwrap_or_else(|| {
        if ran.timed_out() {
            format!("No answer within {} s", ANSWER_LIMIT.as_secs())
        } else if text.trim().is_empty() {
            ran.last_words()
        } else {
            format!("Replied “{}”", run::shorten(text.trim(), 80))
        }
    });
    let fix = if signed_out(&problem) {
        Some(sign_in_fix(backend))
    } else if backend == AssistantBackend::OpenCode && !crate::desktop::opencode_service::running()
    {
        Some(Fix::StartOpenCode)
    } else {
        Some(Fix::Retry)
    };
    CheckResult::failed(check, problem, fix)
}

pub(super) fn sign_in_fix(backend: AssistantBackend) -> Fix {
    match backend {
        AssistantBackend::ClaudeApi => Fix::AddApiKey,
        backend => Fix::SignIn(backend),
    }
}

/// Whether an agent's error says it isn't signed in.
pub fn signed_out(problem: &str) -> bool {
    let problem = problem.to_ascii_lowercase();
    [
        "not logged in",
        "please run /login",
        "/login",
        "log in",
        "login",
        "sign in",
        "invalid api key",
        "unauthorized",
        "401",
        "authentication",
        "oauth token",
    ]
    .iter()
    .any(|marker| problem.contains(marker))
}

/// `codex exec`, read-only, without a repository or saved session.
pub(super) fn codex_base(folder: &Path) -> Vec<String> {
    [
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ephemeral",
        "--sandbox",
        "read-only",
        "--cd",
    ]
    .map(str::to_owned)
    .into_iter()
    .chain([folder.to_string_lossy().into_owned()])
    .collect()
}

/// One JSON value a line, as each agent's event stream prints them.
pub(super) fn parse_events(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect()
}

/// The agent's reply text, from any of the three event streams.
pub fn answer_text(events: &[Value]) -> String {
    let mut text = String::new();
    for event in events {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            // claude -p: the final result.
            "result" => {
                if let Some(result) = event.get("result").and_then(Value::as_str) {
                    return result.to_owned();
                }
            }
            // codex exec --json
            "item.completed" => {
                let item = &event["item"];
                if item.get("type").and_then(Value::as_str) == Some("agent_message")
                    && let Some(message) = item.get("text").and_then(Value::as_str)
                {
                    text = message.to_owned();
                }
            }
            // opencode run --format json
            "text" => {
                if let Some(part) = event["part"].get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            _ => {}
        }
    }
    text
}

/// The first error an event stream reports, for a person.
pub fn event_error(events: &[Value]) -> Option<String> {
    events.iter().find_map(|event| {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = match kind {
            // claude -p
            "result" if event.get("is_error").and_then(Value::as_bool) == Some(true) => event
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    event
                        .get("subtype")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
            // codex: `error` events and a failed turn; opencode: `error`.
            "error" => event
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| event["error"].get("message").and_then(Value::as_str))
                .map(str::to_owned),
            "turn.failed" => event["error"]
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }?;
        Some(run::shorten(message.trim(), 240))
    })
}

pub(super) fn api_answers(check: Check, settings: &AssistantSettings) -> CheckResult {
    let Some(key) = crate::assistant::credentials::api_key() else {
        return CheckResult::failed(check, "No Anthropic API key is saved", Some(Fix::AddApiKey));
    };
    const URL: &str = "https://api.anthropic.com/v1/messages";
    let agent = crate::net::agent(URL, |config| {
        config
            .timeout_global(Some(ANSWER_LIMIT))
            .http_status_as_error(false)
    });
    let body = serde_json::json!({
        "model": settings.claude_api_model,
        "max_tokens": 16,
        "messages": [{ "role": "user", "content": ANSWER_PROMPT }],
    });
    let started = Instant::now();
    let response = agent
        .post(URL)
        .header("x-api-key", key.as_str())
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .send(body.to_string());
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            return CheckResult::failed(
                check,
                crate::net::unreachable(URL, &error),
                Some(Fix::Retry),
            );
        }
    };
    let status = response.status().as_u16();
    let reply: Value = response
        .body_mut()
        .read_to_string()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    if status != 200 {
        let message = reply["error"]["message"]
            .as_str()
            .unwrap_or("the API refused")
            .to_owned();
        let fix = if status == 401 || status == 403 {
            Fix::AddApiKey
        } else {
            Fix::Retry
        };
        return CheckResult::failed(check, format!("{status}: {message}"), Some(fix));
    }
    let text = reply["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>();
    if text.to_ascii_uppercase().contains("OK") {
        CheckResult::passed(
            check,
            format!(
                "Replied “{}” in {:.1} s",
                run::shorten(text.trim(), 40),
                started.elapsed().as_secs_f32()
            ),
        )
    } else {
        CheckResult::failed(
            check,
            format!("Replied “{}”", run::shorten(text.trim(), 80)),
            Some(Fix::Retry),
        )
    }
}
