//! The hidden browser's self-tests: agent-browser opens a local `data:`
//! page with a made-up title in a session of its own, the title is read
//! back, and the session is closed. Nothing on the web is visited and no
//! window shows.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{
    Check, CheckResult, Fix, agent_command, answer_text, codex_base, event_error, installed, name,
    parse_events, run, scratch, sign_in_fix, signed_out,
};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::backends::McpServer;
use crate::assistant::browser::{self, MCP_NAME};

pub const ROUND_TRIP_LIMIT: Duration = Duration::from_secs(60);
pub const THROUGH_AGENT_LIMIT: Duration = Duration::from_secs(90);

/// A check's page: its title is `Agent-Desk-check-<nonce>`, spelled `%2D` in the
/// URL, so finding the title in an agent's output proves the page was read,
/// not just that the URL was repeated.
pub struct Page {
    pub url: String,
    pub title: String,
    /// The check's own session (`browser::session` of this).
    pub task: String,
}

impl Page {
    pub fn new() -> Self {
        let nonce: String = crate::assistant::store::new_id()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(10)
            .collect();
        Self {
            url: format!("data:text/html,<title>Agent-Desk%2Dcheck%2D{nonce}</title>"),
            title: format!("Agent-Desk-check-{nonce}"),
            task: format!("check-{nonce}"),
        }
    }
}

/// agent-browser, or the check failing with its install fix.
fn agent_browser(check: Check) -> Result<std::path::PathBuf, CheckResult> {
    browser::executable().ok_or_else(|| {
        CheckResult::failed(
            check,
            "agent-browser isn't installed",
            Some(Fix::InstallAgentBrowser),
        )
    })
}

pub fn round_trip(check: Check) -> CheckResult {
    let path = match agent_browser(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let page = Page::new();
    let session = browser::session(&page.task);
    let started = Instant::now();
    let opened = browser::call(&path, &session, &["open", &page.url]);
    let title = opened.and_then(|_| browser::call(&path, &session, &["get", "title"]));
    browser::close_now(&page.task);
    match title {
        Ok(data) if data.get("title").and_then(Value::as_str) == Some(page.title.as_str()) => {
            CheckResult::passed(
                check,
                format!(
                    "Opened a hidden page and read its title in {:.1} s",
                    started.elapsed().as_secs_f32()
                ),
            )
        }
        Ok(data) => CheckResult::failed(
            check,
            format!(
                "agent-browser read the wrong title: {}",
                data.get("title").unwrap_or(&Value::Null)
            ),
            Some(Fix::Retry),
        ),
        Err(error) => {
            let fix = if !browser::agent_browser().browser_ready {
                Fix::InstallAgentBrowser
            } else {
                Fix::Retry
            };
            CheckResult::failed(
                check,
                format!("agent-browser couldn't open a page: {error}"),
                Some(fix),
            )
        }
    }
}

/// Lightpanda opens the check's page through agent-browser.
pub fn light_round_trip(check: Check) -> CheckResult {
    let path = match agent_browser(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    if browser::lightpanda().is_none() {
        return CheckResult::failed(
            check,
            "Lightpanda isn't installed, so clean browsers run Chrome (about ten times the memory)",
            Some(Fix::InstallLightpanda),
        );
    }
    let page = Page::new();
    let session = browser::session(&page.task);
    let started = Instant::now();
    let engine = ["--engine", "lightpanda"];
    let opened = browser::call(
        &path,
        &session,
        &[&engine[..], &["open", &page.url]].concat(),
    );
    let title = opened.and_then(|_| browser::call(&path, &session, &["get", "title"]));
    browser::close_now(&page.task);
    match title {
        Ok(data) if data.get("title").and_then(Value::as_str) == Some(page.title.as_str()) => {
            CheckResult::passed(
                check,
                format!(
                    "Opened a page in Lightpanda in {:.1} s",
                    started.elapsed().as_secs_f32()
                ),
            )
        }
        Ok(data) => CheckResult::failed(
            check,
            format!(
                "Lightpanda read the wrong title: {}",
                data.get("title").unwrap_or(&Value::Null)
            ),
            Some(Fix::Retry),
        ),
        Err(error) => CheckResult::failed(
            check,
            format!("Lightpanda couldn't open a page: {error}"),
            Some(Fix::InstallLightpanda),
        ),
    }
}

/// The prompt that has an agent open the page with its own name for the
/// tool and say the title.
fn open_prompt(tool: &str, url: &str) -> String {
    format!(
        "Call the {tool} tool once, with url set to exactly {url}. Then reply with only the \
         page title it reports. Do nothing else."
    )
}

pub fn through_agent(
    check: Check,
    backend: AssistantBackend,
    settings: &AssistantSettings,
) -> CheckResult {
    if let Err(failed) = agent_browser(check) {
        return failed;
    }
    let page = Page::new();
    let Some(server) = browser::mcp_server(&page.task) else {
        return CheckResult::failed(
            check,
            "Agent-Desk couldn't start its browser tool",
            Some(Fix::Retry),
        );
    };
    let result = match backend {
        AssistantBackend::ClaudeApi => through_hex(check, &server, &page),
        backend => through(check, backend, settings, &server, &page),
    };
    browser::close_now(&page.task);
    result
}

/// The agent, given only the browser tool's `open`.
fn through(
    check: Check,
    backend: AssistantBackend,
    settings: &AssistantSettings,
    server: &McpServer,
    page: &Page,
) -> CheckResult {
    let path = match installed(check, backend) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let mut command = agent_command(&path);
    command.current_dir(scratch());
    match backend {
        AssistantBackend::ClaudeCode => {
            let tool = format!("mcp__{MCP_NAME}__agent_browser_open");
            command.args([
                "-p",
                &open_prompt(&tool, &page.url),
                "--output-format",
                "stream-json",
            ]);
            command.args(["--verbose", "--tools", "", "--no-session-persistence"]);
            command.args([
                "--mcp-config",
                &server.claude_config_as(MCP_NAME),
                "--strict-mcp-config",
            ]);
            command.args(["--allowedTools", &tool]);
            if let Some(model) = &settings.claude_code_model {
                command.args(["--model", model]);
            }
        }
        AssistantBackend::Codex => {
            command.args(codex_base(&scratch()));
            command.args(server.codex_args_as(MCP_NAME, &["agent_browser_open"]));
            if let Some(model) = &settings.codex_model {
                command.args(["--model", model]);
            }
            command.arg(open_prompt("agent_browser_open", &page.url));
        }
        AssistantBackend::OpenCode => {
            let tool = format!("{MCP_NAME}_agent_browser_open");
            let config = json!({
                "mcp": server.opencode_config_as(MCP_NAME),
                "tools": { "*": false, tool.as_str(): true },
            });
            let model = crate::assistant::backends::providers::opencode_check(
                &mut command,
                settings,
                config,
            );
            command.args(["run", "--standalone", "--format", "json", "--archived"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(open_prompt(&tool, &page.url));
        }
        AssistantBackend::ClaudeApi => unreachable!("Agent-Desk calls the tool itself"),
    }
    let ran = match run::bounded(command, THROUGH_AGENT_LIMIT) {
        Ok(ran) => ran,
        Err(error) => {
            return CheckResult::failed(
                check,
                format!("{} didn't start: {error}", name(backend)),
                Some(Fix::Retry),
            );
        }
    };
    if ran.stdout.contains(&page.title) {
        return CheckResult::passed(
            check,
            format!(
                "Opened a hidden page through agent-browser in {:.1} s",
                ran.took.as_secs_f32()
            ),
        );
    }
    let events = parse_events(&ran.stdout);
    let problem = event_error(&events).unwrap_or_else(|| {
        if ran.timed_out() {
            format!("No reply within {} s", THROUGH_AGENT_LIMIT.as_secs())
        } else {
            let said = answer_text(&events);
            if said.trim().is_empty() {
                format!("It never opened the page. {}", ran.last_words())
            } else {
                format!(
                    "It never opened the page; it said “{}”",
                    run::shorten(said.trim(), 80)
                )
            }
        }
    });
    let fix = if signed_out(&problem) {
        sign_in_fix(backend)
    } else {
        Fix::Retry
    };
    CheckResult::failed(check, problem, Some(fix))
}

/// Agent-Desk's own MCP session with the browser tool, as the Claude API's tasks
/// have it: open the page, read its title.
fn through_hex(check: Check, server: &McpServer, page: &Page) -> CheckResult {
    let started = Instant::now();
    match title_over_mcp(server, &page.url, started + THROUGH_AGENT_LIMIT) {
        Ok(text) if text.contains(&page.title) => CheckResult::passed(
            check,
            format!(
                "Agent-Desk opened a hidden page through its browser tool in {:.1} s",
                started.elapsed().as_secs_f32()
            ),
        ),
        Ok(text) => CheckResult::failed(
            check,
            format!(
                "The browser tool read the wrong title: “{}”",
                run::shorten(text.trim(), 80)
            ),
            Some(Fix::Retry),
        ),
        Err(error) => CheckResult::failed(
            check,
            format!("The browser tool failed: {error}"),
            Some(Fix::Retry),
        ),
    }
}

/// Starts `server`, opens `url` with `agent_browser_open`, and returns the
/// text of `agent_browser_get_title`'s result.
pub fn title_over_mcp(server: &McpServer, url: &str, deadline: Instant) -> Result<String, String> {
    let mut command = crate::assistant::backends::search::command(&server.command);
    command
        .args(&server.args)
        .env("PATH", &server.path_env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    run::own_group(&mut command, true);
    let mut child = command
        .spawn()
        .map_err(|error| format!("it didn't start: {error}"))?;
    let tree = run::Tree::of(&child);
    let mut input = child.stdin.take().ok_or("no input")?;
    let output = child.stdout.take().ok_or("no output")?;
    let (lines, replies) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut ask = |id: u64, method: &str, params: Value| -> Result<Value, String> {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(input, "{request}")
            .and_then(|()| input.flush())
            .map_err(|error| format!("it stopped: {error}"))?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = replies
                .recv_timeout(left)
                .map_err(|_| format!("no answer to {method} in time"))?;
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(format!("{method}: {error}"));
            }
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
    };
    let result = (|| {
        ask(
            1,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "hex-check", "version": "1"}}),
        )?;
        let open = ask(
            2,
            "tools/call",
            json!({"name": "agent_browser_open", "arguments": {"url": url}}),
        )?;
        if open.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(format!("open failed: {}", text_of(&open)));
        }
        let title = ask(
            3,
            "tools/call",
            json!({"name": "agent_browser_get_title", "arguments": {}}),
        )?;
        Ok(text_of(&title))
    })();
    drop(input);
    tree.kill_with(&mut child);
    result
}

/// A tool result's text blocks, joined.
fn text_of(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_page_hides_its_title_in_its_url() {
        let page = Page::new();
        assert!(page.title.starts_with("Agent-Desk-check-"));
        assert!(!page.url.contains(&page.title));
        assert!(
            page.url
                .starts_with("data:text/html,<title>Agent-Desk%2Dcheck%2D")
        );
        assert_eq!(browser::session(&page.task), format!("hex-{}", page.task));
        assert!(open_prompt("agent_browser_open", &page.url).contains(&page.url));
    }

    #[test]
    fn a_tool_results_text_is_joined() {
        let result =
            json!({"content": [{"type": "text", "text": "Agent-Desk-check-a"}, {"type": "image"}]});
        assert_eq!(text_of(&result), "Agent-Desk-check-a");
    }
}
