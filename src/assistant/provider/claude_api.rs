//! The Claude Messages API, with the key from the Keychain
//! (`credentials::api_key`). Hex runs the tool loop itself: each turn's
//! tool calls go to the task's MCP servers (`mcp`: its desktop tool and its
//! browser, whichever offers the tool), their results go
//! back in one user message, and the loop ends at an answer or after
//! `MAX_STEPS` turns. The API keeps no session, so a conversation continues
//! from its saved messages.

use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::desktop::Describer;
use super::mcp::{Called, Client};
use super::{Control, DESKTOP_TOOLS, Finished, Report, Run};
use crate::assistant::types::{Message, Role};

const URL: &str = "https://api.anthropic.com/v1/messages";
const VERSION: &str = "2026-10-21";
const MAX_STEPS: usize = 40;
const MAX_TOKENS: u32 = 16_000;

pub(super) fn run(
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let key = crate::assistant::credentials::api_key().ok_or_else(|| {
        eyre!(
            "No Anthropic API key is saved. Add one in {}, or choose another agent.",
            crate::assistant::lines::ASSISTANT_SETTINGS
        )
    })?;
    let mut servers = Vec::new();
    let mut tools = Vec::new();
    let started = (|| -> Result<()> {
        for (name, server) in super::servers::of(run) {
            let mut client = Client::start(server, run.desktop.as_ref(), &run.workdir, control)?;
            let (definitions, names) = tool_definitions(&mut client, name)?;
            tools.extend(definitions);
            servers.push(Server {
                client,
                tools: names,
            });
        }
        Ok(())
    })();
    if let Some(last) = tools.last_mut() {
        last["cache_control"] = json!({"type": "ephemeral"});
    }
    let result = started.and_then(|()| run_loop(run, &key, &tools, &mut servers, control, report));
    for server in servers {
        server.client.close(control);
    }
    result
}

/// One MCP server the loop calls, and the tools it offers.
struct Server {
    client: Client,
    tools: Vec<String>,
}

fn run_loop(
    run: &Run,
    key: &str,
    tools: &[Value],
    servers: &mut [Server],
    control: &Control,
    report: &mut dyn FnMut(Report),
) -> Result<Finished> {
    let model = run
        .model
        .clone()
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| "claude-sonnet-5-5".into());
    let mut messages = history(&run.history);
    messages.push(json!({"role": "user", "content": first_content(run)?}));
    let mut describer = Describer::default();
    let mut wrote = false;
    for _ in 0..MAX_STEPS {
        if control.cancelled() {
            return Err(eyre!("cancelled"));
        }
        report(Report::Thinking);
        let body = request(&model, &run.system, tools, &messages, run.quick);
        let response = send(key, &body)?;
        if control.cancelled() {
            return Err(eyre!("cancelled"));
        }
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = content
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
        if !text.trim().is_empty() {
            if wrote {
                report(Report::Delta("\n\n".into()));
            }
            report(Report::Delta(text.trim().to_owned()));
            wrote = true;
        }
        let stop = response
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        messages.push(json!({"role": "assistant", "content": content}));
        match stop {
            "tool_use" => {
                let results = run_tools(&content, servers, &mut describer, report)?;
                messages.push(json!({"role": "user", "content": results}));
            }
            // A long server-side turn paused: send the conversation again.
            "pause_turn" => {}
            "refusal" => return Err(eyre!("Claude declined to answer this.")),
            "max_tokens" if text.trim().is_empty() => {
                return Err(eyre!("The answer was too long and was cut off."));
            }
            _ => {
                return Ok(Finished {
                    text: text.trim().to_owned(),
                });
            }
        }
    }
    Err(eyre!(
        "It took more than {MAX_STEPS} steps, so Hex stopped it."
    ))
}

/// The request body for one turn.
fn request(model: &str, system: &str, tools: &[Value], messages: &[Value], quick: bool) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    if adaptive(model) {
        body["thinking"] = json!({"type": "adaptive"});
        body["output_config"] = json!({"effort": if quick { "low" } else { "medium" }});
    }
    if fallbacks(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// Whether `model` takes adaptive thinking and effort (Claude 4.6 and later).
fn adaptive(model: &str) -> bool {
    ![
        "-3-",
        "-4-0",
        "-4-1",
        "-4-5",
        "claude-3",
        "claude-opus-4-",
        "claude-sonnet-4-",
    ]
    .iter()
    .any(|old| model.contains(old))
        || model.contains("-4-6")
        || model.contains("-4-7")
        || model.contains("-4-8")
}

/// Whether `model` takes the server-side refusal fallback.
fn fallbacks(model: &str) -> bool {
    [
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-sonnet-5-5",
        "claude-fable-5-1",
    ]
    .contains(&model)
}

/// Whether `said` answers `question` (asked by the task `title`) or starts
/// something new, as `model` judges within `within`; `None` without a key,
/// on an error, or when it takes longer (`service::inbox`).
pub fn classify_reply(
    model: &str,
    question: &str,
    title: &str,
    said: &str,
    within: Duration,
) -> Option<bool> {
    let key = crate::assistant::credentials::api_key()?;
    let body = json!({
        "model": model,
        "max_tokens": 4,
        "system": "A task is waiting for the person's answer to its question. Decide whether what the person said next answers that question, or is a new, unrelated request. Reply with exactly one word: ANSWER or NEW.",
        "messages": [{
            "role": "user",
            "content": format!("Task: {title}\nIts question: {question}\nThe person said: {said}"),
        }],
    });
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("assistant-classify".into())
        .spawn(move || {
            let _ = sender.send(send(&key, &body));
        })
        .ok()?;
    let reply = receiver.recv_timeout(within).ok()?.ok()?;
    let word = reply
        .pointer("/content/0/text")?
        .as_str()?
        .to_ascii_uppercase();
    if word.contains("NEW") {
        Some(false)
    } else if word.contains("ANSWER") {
        Some(true)
    } else {
        None
    }
}

fn send(key: &str, body: &Value) -> Result<Value> {
    let agent = crate::net::agent(URL, |config| {
        config
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(300)))
    });
    let mut request = agent
        .post(URL)
        .header("x-api-key", key)
        .header("anthropic-version", VERSION)
        .header("content-type", "application/json");
    if body.get("fallbacks").is_some() {
        request = request.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    let mut response = request
        .send(body.to_string())
        .map_err(|error| eyre!("{}", crate::net::unreachable(URL, &error)))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let text = response
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_string()
        .map_err(|error| eyre!("couldn't read the Claude API's answer: {error}"))?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status != 200 {
        let why = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no reason given");
        return Err(match status {
            401 => eyre!("The Claude API refused the API key: {why}"),
            429 | 529 => match retry_after {
                Some(seconds) => eyre!(
                    "The Claude API is rate-limiting this key ({status}); retry after {seconds} seconds."
                ),
                None => eyre!("The Claude API is rate-limiting this key ({status}): {why}"),
            },
            _ => eyre!("The Claude API answered {status}: {why}"),
        });
    }
    Ok(value)
}

/// A server's tools as API tool definitions, and their names: the desktop's
/// `DESKTOP_TOOLS`, every tool of the others.
fn tool_definitions(client: &mut Client, server: &str) -> Result<(Vec<Value>, Vec<String>)> {
    let desktop = server == crate::assistant::backends::MCP_NAME;
    let offered: Vec<_> = client
        .tools()?
        .into_iter()
        .filter(|tool| !desktop || DESKTOP_TOOLS.contains(&tool.name.as_str()))
        .collect();
    let names = offered.iter().map(|tool| tool.name.clone()).collect();
    let tools = offered
        .into_iter()
        .map(|tool| json!({"name": tool.name, "description": tool.description, "input_schema": tool.schema}))
        .collect();
    Ok((tools, names))
}

/// Runs every tool call in `content` and returns their results, in order.
fn run_tools(
    content: &[Value],
    servers: &mut [Server],
    describer: &mut Describer,
    report: &mut dyn FnMut(Report),
) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
        report(Report::tool(name, &input));
        for item in describer.call(name, &input) {
            report(item);
        }
        let called = match servers
            .iter_mut()
            .find(|server| server.tools.iter().any(|tool| tool == name))
        {
            Some(server) => server.client.call(name, &input),
            None => Err(eyre!("there is no tool “{name}”")),
        };
        let result = match called {
            Ok(Called { content, is_error }) => {
                let text: String = content
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect();
                for item in describer.result(name, &text) {
                    report(item);
                }
                json!({"type": "tool_result", "tool_use_id": id,
                    "content": result_content(&content), "is_error": is_error})
            }
            Err(error) => json!({"type": "tool_result", "tool_use_id": id,
                "content": format!("{error:#}"), "is_error": true}),
        };
        results.push(result);
    }
    Ok(results)
}

/// MCP content as API tool-result content: text as text, images inline,
/// and a screenshot agent-desktop saved to a file shown as the image.
fn result_content(content: &[Value]) -> Vec<Value> {
    let mut blocks = Vec::new();
    for item in content {
        match item.get("type").and_then(Value::as_str) {
            Some("image") => {
                if let Some(data) = item.get("data").and_then(Value::as_str) {
                    let media = item
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    blocks.push(json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": data}}));
                }
            }
            _ => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                blocks.push(json!({"type": "text", "text": text}));
                if let Some(png) = screenshot_file(text) {
                    blocks.push(json!({"type": "image", "source": {"type": "base64",
                        "media_type": "image/png", "data": super::base64(&png)}}));
                }
            }
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": "(no output)"}));
    }
    blocks
}

fn screenshot_file(text: &str) -> Option<Vec<u8>> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("command").and_then(Value::as_str) != Some("screenshot") {
        return None;
    }
    let path = value.pointer("/data/path").and_then(Value::as_str)?;
    let png = std::fs::read(path).ok()?;
    (png.len() < 8 * 1024 * 1024).then_some(png)
}

/// The first user message: the prompt and the screenshot.
fn first_content(run: &Run) -> Result<Vec<Value>> {
    let mut content = Vec::new();
    if let Some(image) = &run.image {
        let png =
            std::fs::read(image).map_err(|error| eyre!("couldn't read the screenshot: {error}"))?;
        content.push(json!({"type": "image", "source": {"type": "base64",
            "media_type": "image/png", "data": super::base64(&png)}}));
    }
    content.push(json!({"type": "text", "text": run.prompt}));
    Ok(content)
}

/// Earlier messages as API turns: text only, starting with the person and
/// alternating.
fn history(messages: &[Message]) -> Vec<Value> {
    let mut turns: Vec<(Role, String)> = Vec::new();
    for message in messages {
        let text = message.text();
        if text.trim().is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((role, joined)) if *role == message.role => {
                joined.push_str("\n\n");
                joined.push_str(&text);
            }
            _ => turns.push((message.role, text)),
        }
    }
    while turns
        .first()
        .is_some_and(|(role, _)| *role == Role::Assistant)
    {
        turns.remove(0);
    }
    // The new question follows as a user turn, so history ends with Claude.
    if turns.last().is_some_and(|(role, _)| *role == Role::User) {
        turns.pop();
    }
    turns
        .into_iter()
        .map(|(role, text)| {
            let role = if role == Role::User {
                "user"
            } else {
                "assistant"
            };
            json!({"role": role, "content": text})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::types::Part;

    fn message(role: Role, text: &str) -> Message {
        Message {
            role,
            parts: vec![Part::Text { text: text.into() }],
            at_ms: 0,
        }
    }

    #[test]
    fn history_alternates_and_leaves_room_for_the_new_question() {
        let turns = history(&[
            message(Role::Assistant, "stray"),
            message(Role::User, "a"),
            message(Role::User, "b"),
            message(Role::Assistant, "c"),
            message(Role::User, "unanswered"),
        ]);
        assert_eq!(
            turns,
            [
                json!({"role": "user", "content": "a\n\nb"}),
                json!({"role": "assistant", "content": "c"}),
            ]
        );
    }

    #[test]
    fn requests_use_adaptive_thinking_only_where_models_take_it() {
        let quick = request("claude-sonnet-5-5", "sys", &[], &[], true);
        assert_eq!(quick["thinking"], json!({"type": "adaptive"}));
        assert_eq!(quick["output_config"]["effort"], "low");
        assert_eq!(quick["fallbacks"], "default");
        assert!(quick.get("tools").is_none());
        let old = request(
            "claude-haiku-4-5",
            "sys",
            &[json!({"name": "t"})],
            &[],
            false,
        );
        assert!(old.get("thinking").is_none());
        assert!(old.get("fallbacks").is_none());
        assert_eq!(old["tools"][0]["name"], "t");
        assert!(adaptive("claude-opus-4-6"));
        assert!(adaptive("claude-haiku-5-5"));
    }
}
