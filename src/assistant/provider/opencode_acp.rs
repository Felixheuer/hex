//! OpenCode's quick turns over ACP (Agent Client Protocol, `opencode acp`:
//! JSON-RPC on stdin and stdout): one process that can be started ahead
//! while the person still speaks, streams the answer as it is written (so
//! it can be read aloud early), and keeps its session for the follow-up.
//! `opencode run` starts OpenCode for every turn (about 3 s before the
//! model is even asked) and writes each text part only once it is whole.
//!
//! The process gets the same configuration a `run` does
//! (`OPENCODE_CONFIG_CONTENT`: Agent-Desk's agent, made the default, the
//! task's MCP servers, the model, the person's own key), so a session here
//! is the one `opencode run --session` continues: a turn the fast model
//! hands over goes on there with the strong one. Both OpenCodes speak ACP
//! (classic streams token by token, OpenCode 2 in larger parts and through
//! its code mode, `execute`).
//!
//! Anything that keeps ACP from starting (an OpenCode without it, a
//! protocol it answers differently) returns [`Start::Unavailable`], and the
//! turn runs through `opencode run` instead; once the question is asked,
//! a failure is the turn's.

use std::collections::BTreeSet;
use std::io::Write;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use serde_json::{Value, json};

use super::desktop::Describer;
use super::{Control, Finished, Process, Report, Run, Stdin, warm};
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::opencode::{self as cli, Generation};

/// What starting ACP for a turn came to.
pub(super) enum Start {
    /// It ran: the turn's own result.
    Ran(Result<Finished>),
    /// ACP isn't there or didn't come up: run the turn another way.
    Unavailable(String),
}

/// Whether `run` goes through ACP: a quick turn, not a batch-file OpenCode
/// (Windows can't pass it the config's quotes reliably either way).
pub(super) fn fits(run: &Run) -> bool {
    run.quick
}

/// Runs `run` over ACP: on a process started ahead for it when one waits,
/// else on a new one.
pub(super) fn run(run: &Run, control: &Control, report: &mut dyn FnMut(Report)) -> Start {
    let key = warm_key(run);
    wait_for_start(&key);
    let waiting = match warm::take_with(&key, run.resume.as_deref()) {
        Some((process, Some(session))) => {
            tracing::debug!("the assistant uses an OpenCode that is already running");
            control.track(&process.tree);
            Some(Acp {
                process,
                // Above what `start` used.
                next: 100,
                session,
            })
        }
        Some((process, None)) => {
            end(
                Acp {
                    process,
                    next: 0,
                    session: String::new(),
                },
                control,
            );
            None
        }
        None => None,
    };
    let fresh = || -> std::result::Result<Acp, Start> {
        if unavailable() {
            return Err(Start::Unavailable("ACP didn't start before".into()));
        }
        start(run, control).map_err(|failed| {
            if failed.lasting && !control.cancelled() {
                mark_unavailable();
            }
            Start::Unavailable(format!("{:#}", failed.why))
        })
    };
    let (mut acp, reused) = match waiting {
        Some(acp) => (acp, true),
        None => match fresh() {
            Ok(acp) => (acp, false),
            Err(start) => return start,
        },
    };
    report(Report::Session(acp.session.clone()));
    let result = match turn(&mut acp, run, control, report, reused) {
        Some(result) => result,
        None => {
            // The waiting process had ended before it read the question.
            tracing::info!("the waiting OpenCode had ended; starting a new one");
            end(acp, control);
            acp = match fresh() {
                Ok(acp) => acp,
                Err(start) => return start,
            };
            report(Report::Session(acp.session.clone()));
            turn(&mut acp, run, control, report, false)
                .unwrap_or_else(|| Err(eyre!("OpenCode stopped before it answered")))
        }
    };
    match result {
        Ok(_) => keep_or_end(acp, run, &key, control),
        Err(_) => end(acp, control),
    }
    Start::Ran(result)
}

/// Processes being started ahead, by key: a request for one waits for it
/// instead of starting a second, and one whose reservation went void
/// (`void`) ends as soon as it is up.
static STARTING: std::sync::Mutex<Vec<(String, bool)>> = std::sync::Mutex::new(Vec::new());

fn starting() -> std::sync::MutexGuard<'static, Vec<(String, bool)>> {
    STARTING.lock().unwrap_or_else(|error| error.into_inner())
}

/// Waits (a few seconds at most) for a process still being started ahead
/// for `key`.
fn wait_for_start(key: &str) {
    let until = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < until && starting().iter().any(|(started, _)| started == key)
    {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Voids the processes being started ahead for `task` (every one when
/// `task` is empty): the reservation they were for won't be taken.
pub(super) fn void(task: &str) {
    for (key, void) in starting().iter_mut() {
        if task.is_empty() || key.contains(task) {
            *void = true;
        }
    }
}

/// The OpenCode ACP didn't start for, until it changes on disk or
/// Agent-Desk restarts: its turns go through `opencode run` without trying.
static UNAVAILABLE: std::sync::Mutex<Option<(std::path::PathBuf, Option<std::time::SystemTime>)>> =
    std::sync::Mutex::new(None);

fn current() -> Option<(std::path::PathBuf, Option<std::time::SystemTime>)> {
    let path = crate::assistant::backends::executable(AssistantBackend::OpenCode)?;
    let modified = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .ok();
    Some((path, modified))
}

fn unavailable() -> bool {
    let marked = UNAVAILABLE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    marked.is_some() && marked == current()
}

fn mark_unavailable() {
    *UNAVAILABLE
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = current();
}

/// Starts OpenCode's ACP for `run` with its session open, and leaves it
/// waiting for the question, at most `ttl`.
pub(super) fn prewarm(run: &Run, ttl: Duration) {
    let key = warm_key(run);
    {
        let mut starting = starting();
        if unavailable()
            || warm::has(&key, run.resume.as_deref())
            || starting.iter().any(|(started, _)| *started == key)
        {
            return;
        }
        starting.push((key.clone(), false));
    }
    let control = Control::default();
    let started = start(run, &control);
    let void = {
        let mut starting = starting();
        let at = starting.iter().position(|(started, _)| *started == key);
        at.map(|at| starting.remove(at).1).unwrap_or(true)
    };
    match started {
        Ok(acp) if void => {
            // Its reservation went while it started: no one takes it.
            if run.resume.is_none() {
                cli::forget_sessions(vec![acp.session.clone()]);
            }
            end(acp, &control);
        }
        Ok(acp) => warm::put_with(key, run.resume.clone(), acp.process, ttl, Some(acp.session)),
        Err(failed) => {
            if failed.lasting {
                mark_unavailable();
            }
            tracing::debug!(error = %format!("{:#}", failed.why), "couldn't start OpenCode ahead");
        }
    }
}

/// Why ACP didn't start, and whether that lasts (this OpenCode has no ACP
/// Agent-Desk can use) or was this once (slow, a session it couldn't load).
struct Failed {
    why: color_eyre::Report,
    lasting: bool,
}

impl Failed {
    fn once(why: color_eyre::Report) -> Self {
        Self {
            why,
            lasting: false,
        }
    }
}

/// One `opencode acp` with its session open.
struct Acp {
    process: Process,
    next: u64,
    session: String,
}

/// How long OpenCode may take to come up and open a session.
const START_LIMIT: Duration = Duration::from_secs(30);

fn start(run: &Run, control: &Control) -> std::result::Result<Acp, Failed> {
    let executable = super::executable(AssistantBackend::OpenCode).map_err(Failed::once)?;
    let generation = cli::generation_for(&executable);
    if super::opencode::is_batch_file(&crate::assistant::backends::search::launch(&executable).0) {
        return Err(Failed {
            why: eyre!("OpenCode is a batch file here"),
            lasting: true,
        });
    }
    let mut command = super::command(&executable, &run.workdir);
    if let Some(env) = &run.backend_env {
        command
            .env_clear()
            .envs(env.iter().map(|(key, value)| (key, value)));
    }
    command.env(
        "OPENCODE_CONFIG_CONTENT",
        config(run, generation).to_string(),
    );
    command.arg("acp");
    let process = Process::spawn(command, Stdin::Keep, control).map_err(Failed::once)?;
    let mut acp = Acp {
        process,
        next: 1,
        session: String::new(),
    };
    // The process must answer in time; a stuck start is ended.
    let deadline = std::time::Instant::now() + START_LIMIT;
    let tree = acp.process.tree.clone();
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let started = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let (watching, late) = (Arc::clone(&started), Arc::clone(&timed_out));
    std::thread::spawn(move || {
        while std::time::Instant::now() < deadline {
            if watching.load(Ordering::Acquire) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if !watching.load(Ordering::Acquire) {
            late.store(true, Ordering::Release);
            super::end_group(tree);
        }
    });
    let opened = open(&mut acp, run);
    started.store(true, Ordering::Release);
    match opened {
        Ok(session) => {
            acp.session = session;
            Ok(acp)
        }
        Err(mut failed) => {
            // Slow is no lasting reason.
            failed.lasting &= !timed_out.load(Ordering::Acquire);
            super::end_group(acp.process.tree.clone());
            let _ = acp.process.finish(control);
            Err(failed)
        }
    }
}

/// `initialize`, then a new session (or the conversation's, loaded). An
/// OpenCode that ends before it answers `initialize` (no `acp` command), or
/// answers in another protocol, won't do later either; anything after is
/// this once.
fn open(acp: &mut Acp, run: &Run) -> std::result::Result<String, Failed> {
    let init = acp
        .request(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {
                "fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}}),
            &mut |_| {},
        )
        .map_err(|why| Failed { why, lasting: true })?;
    if init.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err(Failed {
            why: eyre!("OpenCode speaks another ACP: {init}"),
            lasting: true,
        });
    }
    let cwd = run.workdir.display().to_string();
    let session = match &run.resume {
        Some(session) => {
            let can_load = init
                .pointer("/agentCapabilities/loadSession")
                .and_then(Value::as_bool)
                == Some(true);
            if !can_load {
                return Err(Failed::once(eyre!(
                    "this OpenCode can't load a session over ACP"
                )));
            }
            // Its history comes back as updates first; they were shown
            // when they happened.
            acp.request(
                "session/load",
                json!({"sessionId": session, "cwd": cwd, "mcpServers": []}),
                &mut |_| {},
            )
            .map_err(Failed::once)?;
            session.clone()
        }
        None => acp
            .request(
                "session/new",
                json!({"cwd": cwd, "mcpServers": []}),
                &mut |_| {},
            )
            .map_err(Failed::once)?
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Failed::once(eyre!("OpenCode opened no session")))?,
    };
    Ok(session)
}

impl Acp {
    fn send(&mut self, message: &Value) -> Result<()> {
        let pipe = self
            .process
            .stdin()
            .ok_or_else(|| eyre!("OpenCode's input is closed"))?;
        let mut line = message.to_string();
        line.push('\n');
        pipe.write_all(line.as_bytes())
            .and_then(|()| pipe.flush())
            .map_err(|error| eyre!("OpenCode stopped listening: {error}"))
    }

    /// Sends `method` and reads until its answer, handing every
    /// notification to `notified` and answering OpenCode's own requests.
    fn request(
        &mut self,
        method: &str,
        params: Value,
        notified: &mut dyn FnMut(&Value),
    ) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        loop {
            let line = self
                .process
                .next_line()
                .ok_or_else(|| eyre!("OpenCode ended before it answered {method}"))?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    let answer = answer_request(&message);
                    self.send(&answer)?;
                } else {
                    notified(&message);
                }
                continue;
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(eyre!("OpenCode: {}", error_words(error)));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// The answer to a request OpenCode makes of Agent-Desk: a permission is
/// granted once, as `opencode run --auto` grants what isn't denied (the
/// agent's own rules deny what a lane may not do); anything else isn't
/// offered.
fn answer_request(request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("session/request_permission") => {
            let options = request
                .pointer("/params/options")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let pick = |kind: &str| {
                options
                    .iter()
                    .find(|option| option.get("kind").and_then(Value::as_str) == Some(kind))
                    .and_then(|option| option.get("optionId").cloned())
            };
            match pick("allow_once").or_else(|| pick("allow_always")) {
                Some(option) => json!({"jsonrpc": "2.0", "id": id,
                    "result": {"outcome": {"outcome": "selected", "optionId": option}}}),
                None => json!({"jsonrpc": "2.0", "id": id,
                    "result": {"outcome": {"outcome": "cancelled"}}}),
            }
        }
        _ => json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": "Agent-Desk doesn't offer this"}}),
    }
}

/// An ACP error's words: its message, and the provider's own when OpenCode
/// passes it on.
fn error_words(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("an error");
    let detail = error
        .pointer("/data/message")
        .or_else(|| error.pointer("/data/error/message"))
        .or_else(|| error.get("data").filter(|data| data.is_string()))
        .and_then(Value::as_str);
    match detail {
        Some(detail) if detail != message => format!("{message}: {detail}"),
        _ => message.to_owned(),
    }
}

/// Asks `run`'s question in the session and reads the answer as it comes.
/// `None` when a waiting process (`reused`) turned out to have ended before
/// it read the question. OpenCode answers the prompt only after its last
/// update (checked with both OpenCodes), so nothing of one turn is left
/// for the next on a kept process.
fn turn(
    acp: &mut Acp,
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
    reused: bool,
) -> Option<Result<Finished>> {
    turn_in(acp, run, control, report, reused).ok()
}

/// A waiting process that had ended.
struct Dead;

fn turn_in(
    acp: &mut Acp,
    run: &Run,
    control: &Control,
    report: &mut dyn FnMut(Report),
    reused: bool,
) -> std::result::Result<Result<Finished>, Dead> {
    let mut lines = 0usize;
    let mut prompt = vec![json!({"type": "text", "text": run.prompt})];
    if let Some(image) = &run.image
        && let Ok(png) = std::fs::read(image)
    {
        prompt.push(json!({"type": "image", "mimeType": "image/png", "data": super::base64(&png)}));
    }
    let mut stream = Stream::default();
    let session = acp.session.clone();
    let after = run.escalate_after;
    let id = acp.next;
    acp.next += 1;
    if let Err(error) = acp.send(
        &json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
        "params": {"sessionId": session, "prompt": prompt}}),
    ) {
        return if reused { Err(Dead) } else { Ok(Err(error)) };
    }
    loop {
        let Some(line) = acp.process.next_line() else {
            if control.cancelled() {
                return Ok(Err(eyre!("cancelled")));
            }
            if reused && lines == 0 {
                return Err(Dead);
            }
            return Ok(Err(eyre!("OpenCode stopped before it answered")));
        };
        lines += 1;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        report(Report::Alive);
        if message.get("method").is_some() {
            if message.get("id").is_some() {
                let answer = answer_request(&message);
                if let Err(error) = acp.send(&answer) {
                    return Ok(Err(error));
                }
            } else if message.get("method").and_then(Value::as_str) == Some("session/update")
                && message.pointer("/params/sessionId").and_then(Value::as_str)
                    == Some(session.as_str())
            {
                stream.update(&message["params"]["update"], report);
                // A quick turn the fast model finds hard: the strong one
                // goes on in this session, between two tools.
                if after.is_some_and(|after| stream.tools >= after) && stream.running.is_empty() {
                    tracing::info!(
                        tools = stream.tools,
                        "the fast model hands the turn to the strong one"
                    );
                    return Ok(Err(super::Escalate {
                        tools: stream.tools,
                    }
                    .into()));
                }
            }
            continue;
        }
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Ok(Err(eyre!("OpenCode: {}", error_words(error))));
        }
        let reason = message
            .pointer("/result/stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn");
        return Ok(match reason {
            "cancelled" => Err(eyre!("cancelled")),
            "refusal" => Err(eyre!("OpenCode: the model refused to answer")),
            _ if stream.answer.trim().is_empty() => {
                Err(eyre!("OpenCode stopped without an answer ({reason})"))
            }
            _ => Ok(Finished {
                text: stream.answer.trim().to_owned(),
            }),
        });
    }
}

/// After an answer: the process waits for the follow-up when `run` asks
/// for that, else it ends.
fn keep_or_end(acp: Acp, run: &Run, key: &str, control: &Control) {
    match run.keep_warm {
        Some(ttl) if !control.cancelled() => {
            control.untrack(acp.process.child.id());
            let session = acp.session.clone();
            warm::put_with(
                key.to_owned(),
                Some(session.clone()),
                acp.process,
                ttl,
                Some(session),
            );
        }
        _ => end(acp, control),
    }
}

/// Ends the process (and its MCP servers), reaped off this thread.
fn end(acp: Acp, control: &Control) {
    let mut process = acp.process;
    drop(process.stdin.take());
    super::end_group(process.tree.clone());
    control.untrack(process.child.id());
    std::thread::spawn(move || {
        let _ = process.child.wait();
    });
}

/// The config the process gets: a `run`'s, with Agent-Desk's agent as the
/// default (ACP has no `--agent`) and the turn's model (nor `--model`).
fn config(run: &Run, generation: Generation) -> Value {
    let mut config = super::opencode::config(run, generation);
    config["default_agent"] = super::opencode::AGENT.into();
    if let Some(model) = super::opencode::model(run) {
        config["model"] = model.into();
    }
    config
}

/// What makes a waiting process fit `run`: the config it was started with
/// (rules, the task's MCP servers, model) and its folder; the session is
/// matched on its own.
fn warm_key(run: &Run) -> String {
    let generation = super::executable(AssistantBackend::OpenCode)
        .map(|path| cli::generation_for(&path))
        .unwrap_or(Generation::Two);
    format!("acp {} {:?}", config(run, generation), run.workdir)
}

/// ACP's session updates, read into reports and the answer.
#[derive(Default)]
struct Stream {
    describer: Describer,
    /// The text written since the last tool call: the answer so far.
    answer: String,
    wrote: bool,
    tool_after_text: bool,
    /// Tool calls that ended.
    tools: usize,
    /// Tool calls started and not ended.
    running: BTreeSet<String>,
    /// Each call's tool, from its start.
    titles: std::collections::HashMap<String, String>,
    /// Calls already shown (audit, steps).
    described: BTreeSet<String>,
    /// The last update was the model's thinking.
    thinking: bool,
}

impl Stream {
    fn update(&mut self, update: &Value, report: &mut dyn FnMut(Report)) {
        let kind = update.get("sessionUpdate").and_then(Value::as_str);
        let was_thinking =
            std::mem::replace(&mut self.thinking, kind == Some("agent_thought_chunk"));
        match kind {
            Some("agent_message_chunk") => {
                let text = update
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                if self.tool_after_text {
                    self.tool_after_text = false;
                    if !self.answer.trim().is_empty() {
                        self.answer.push_str("\n\n");
                    }
                    report(Report::Delta("\n\n".into()));
                }
                self.answer.push_str(text);
                self.wrote = true;
                report(Report::Delta(text.to_owned()));
            }
            // Once as it starts thinking, not for every piece of it.
            Some("agent_thought_chunk") if !was_thinking => report(Report::Thinking),
            Some("tool_call") => {
                let id = update
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if let Some(title) = update.get("title").and_then(Value::as_str) {
                    self.titles.insert(id.clone(), title.to_owned());
                }
                self.running.insert(id);
                self.answer.clear();
                if self.wrote {
                    self.tool_after_text = true;
                }
            }
            Some("tool_call_update") => {
                let status = update.get("status").and_then(Value::as_str);
                let id = update
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                // The tool's name, as its call started (a later title may
                // be a label for people).
                if let Some(title) = update.get("title").and_then(Value::as_str) {
                    self.titles
                        .entry(id.clone())
                        .or_insert_with(|| title.to_owned());
                }
                let tool = self.titles.get(&id).cloned().unwrap_or_default();
                let input = update.get("rawInput").cloned().unwrap_or(Value::Null);
                let ended = matches!(status, Some("completed" | "failed"));
                // The call, once: when its arguments are known, else when
                // it ends.
                let has_input = input.as_object().is_some_and(|input| !input.is_empty());
                if !tool.is_empty()
                    && !self.described.contains(&id)
                    && ((status == Some("in_progress") && has_input) || ended)
                {
                    self.described.insert(id.clone());
                    let part = json!({"tool": tool, "state": {"input": input}});
                    for (name, args) in super::opencode::calls(&part) {
                        report(Report::tool(&name, &args));
                        for item in self.describer.call(&name, &args) {
                            report(item);
                        }
                    }
                }
                if ended && self.running.remove(&id) {
                    self.tools += 1;
                    if let Some(output) =
                        update.pointer("/rawOutput/output").and_then(Value::as_str)
                    {
                        for item in self.describer.result(&tool, output) {
                            report(item);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(stream: &mut Stream, updates: &[Value]) -> Vec<Report> {
        let mut reports = Vec::new();
        for update in updates {
            stream.update(update, &mut |report| reports.push(report));
        }
        reports
    }

    #[test]
    fn chunks_stream_and_the_answer_is_the_text_after_the_last_tool() {
        let mut stream = Stream::default();
        let reports = feed(
            &mut stream,
            &[
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Ich schaue "}}),
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "nach."}}),
                json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "agent-browser_agent_browser_open", "status": "pending", "rawInput": {}}),
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "in_progress", "title": "agent-browser_agent_browser_open", "rawInput": {"url": "https://example.com"}}),
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed", "rawOutput": {"output": "Example Domain"}}),
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Der Titel ist "}}),
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Example Domain."}}),
            ],
        );
        assert_eq!(stream.answer, "Der Titel ist Example Domain.");
        assert_eq!(stream.tools, 1);
        assert!(stream.running.is_empty());
        let deltas: String = reports
            .iter()
            .filter_map(|report| match report {
                Report::Delta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Ich schaue nach.\n\nDer Titel ist Example Domain.");
        assert!(reports.iter().any(|report| matches!(
            report,
            Report::Tool { name, .. } if name == "agent-browser_agent_browser_open"
        )));
    }

    #[test]
    fn a_tool_keeps_the_name_its_call_started_with() {
        let mut stream = Stream::default();
        let reports = feed(
            &mut stream,
            &[
                json!({"sessionUpdate": "tool_call", "toolCallId": "c", "title": "agent-browser_agent_browser_get_title", "rawInput": {}}),
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "c", "status": "completed", "title": "Example Domain", "rawOutput": {"output": "Example Domain"}}),
            ],
        );
        assert!(reports.iter().any(|report| matches!(
            report,
            Report::Tool { name, .. } if name == "agent-browser_agent_browser_get_title"
        )));
        assert_eq!(stream.tools, 1);
    }

    #[test]
    fn thinking_is_reported_once_per_stretch() {
        let mut stream = Stream::default();
        let thought = json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hm"}});
        let chunk = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Ja."}});
        let reports = feed(
            &mut stream,
            &[
                thought.clone(),
                thought.clone(),
                thought.clone(),
                chunk,
                thought,
            ],
        );
        let thinking = reports
            .iter()
            .filter(|report| matches!(report, Report::Thinking))
            .count();
        assert_eq!(thinking, 2);
    }

    #[test]
    fn a_running_tool_holds_the_hand_over() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            &[
                json!({"sessionUpdate": "tool_call", "toolCallId": "a", "title": "x", "rawInput": {}}),
                json!({"sessionUpdate": "tool_call", "toolCallId": "b", "title": "y", "rawInput": {}}),
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "a", "status": "completed"}),
            ],
        );
        assert_eq!(stream.tools, 1);
        assert!(!stream.running.is_empty(), "b still runs");
        feed(
            &mut stream,
            &[json!({"sessionUpdate": "tool_call_update", "toolCallId": "b", "status": "failed"})],
        );
        assert!(stream.running.is_empty());
        assert_eq!(stream.tools, 2);
    }

    #[test]
    fn permissions_are_granted_once_and_other_requests_refused() {
        let asked = json!({"jsonrpc": "2.0", "id": 7, "method": "session/request_permission",
            "params": {"options": [
                {"optionId": "always", "kind": "allow_always"},
                {"optionId": "once", "kind": "allow_once"},
                {"optionId": "no", "kind": "reject_once"}]}});
        assert_eq!(
            answer_request(&asked)["result"]["outcome"],
            json!({"outcome": "selected", "optionId": "once"})
        );
        let other = json!({"jsonrpc": "2.0", "id": 8, "method": "fs/read_text_file", "params": {}});
        assert_eq!(answer_request(&other)["error"]["code"], -32601);
        assert_eq!(answer_request(&other)["id"], 8);
    }

    #[test]
    fn the_config_makes_agent_desks_agent_the_default_with_the_turns_model() {
        let mut run = Run::fixture(AssistantBackend::OpenCode);
        run.quick = true;
        run.model = Some("deepseek/deepseek-flash".into());
        let config = config(&run, Generation::Classic);
        assert_eq!(config["default_agent"], super::super::opencode::AGENT);
        assert_eq!(config["model"], "deepseek/deepseek-flash");
        assert!(config["agent"][super::super::opencode::AGENT].is_object());
        assert!(warm_key(&run).starts_with("acp "));
    }

    #[test]
    fn errors_say_the_providers_words() {
        assert_eq!(
            error_words(&json!({"code": -32603, "message": "Internal error",
                "data": {"message": "Insufficient Balance"}})),
            "Internal error: Insufficient Balance"
        );
        assert_eq!(
            error_words(&json!({"message": "Auth required"})),
            "Auth required"
        );
    }
}
