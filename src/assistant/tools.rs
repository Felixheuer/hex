//! Hex's own small MCP server for a task's agent (`hex assistant tools-mcp
//! --task <id> [--role job]`): `hex_remember` adds a fact to the person's
//! memory note, `hex_schedule`, `hex_schedules`, `hex_unschedule` manage
//! scheduled jobs, `hex_compare_images` tells how alike two screenshots look
//! (a job's own check, `agent::goal`), and `hex_start_jobs` hands a request
//! that is a job to the background (`agent::handoff`). Newline-delimited
//! JSON-RPC on stdio, like the other servers.
//!
//! Two roles. A quick question or a chat (the person's own turn) has all of
//! the above. A background job (`--role job`) reads web pages and files the
//! person never saw, so it may not change what outlives it: no memory, no
//! schedules made or removed, no hand-off (a job's `hex_start_jobs` was lost
//! silently); it leads a team instead (`hex_team_start`, `hex_team_status`,
//! `agent::team`).
//!
//! OWNER: brain.

mod images;

use std::io::{BufRead, Write};

/// What a lead hears when fewer of the helpers it `asked` for started than
/// it asked for (`started`): which wait, and that it starts them later.
fn not_started(asked: &[String], started: usize) -> Option<String> {
    let waiting = asked.get(started..).filter(|rest| !rest.is_empty())?;
    Some(format!(
        "\nNot started, since {started} is as many as this computer runs well beside the others: {}. Start them with hex_team_start as these finish.",
        waiting.join("; ")
    ))
}

use color_eyre::Result;
use serde_json::{Value, json};

use crate::assistant::types::Lane;
use crate::assistant::{memory, schedules};

/// The server's name in each agent's config.
pub const NAME: &str = "hex";

/// The conversation this server serves (`--task`), for `hex_start_jobs`.
static TASK: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// Whether it serves a background job (`--role job`).
static JOB: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Who the tools serve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// A quick question or a chat: the person's own turn.
    Turn,
    /// A background job.
    Job,
}

impl Role {
    /// The `--role` argument's value.
    pub fn arg(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Job => "job",
        }
    }
}

fn role() -> Role {
    if JOB.get().copied().unwrap_or(false) {
        Role::Job
    } else {
        Role::Turn
    }
}

/// Tools a background job may not use, and why.
const NOT_FOR_JOBS: [(&str, &str); 4] = [
    (
        "hex_start_jobs",
        "You are already a background job: start helpers with hex_team_start instead, or do it yourself.",
    ),
    (
        "hex_remember",
        "A background job can't change the person's memory (it may have read something untrusted). Put what's worth remembering in your final answer; the person decides.",
    ),
    (
        "hex_schedule",
        "A background job can't make schedules (it may have read something untrusted). Suggest the schedule in your final answer; the person sets it up.",
    ),
    (
        "hex_unschedule",
        "A background job can't remove the person's schedules. Say in your final answer which one should go.",
    ),
];

/// The tools for `role`.
pub fn definitions_for(role: Role) -> Vec<Value> {
    let all = definitions();
    match role {
        Role::Turn => all,
        Role::Job => all
            .into_iter()
            .filter(|tool| {
                let name = tool["name"].as_str().unwrap_or("");
                !NOT_FOR_JOBS.iter().any(|(refused, _)| *refused == name)
            })
            .chain(team_definitions())
            .collect(),
    }
}

/// A job's team tools (`agent::team`).
fn team_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hex_team_start",
            "description": "Start helpers: background jobs that work beside you on parts of this job, each with its own browsers, apps, and a folder inside yours that you can read. Use it for parts that can run in parallel (many sites to research, several deliverables); pass every helper in one call and they all start at once (as many as this computer runs well; it says if some must wait). Returns their ids. Follow them with hex_team_status.",
            "inputSchema": {"type": "object", "properties": {
                "jobs": {"type": "array", "items": {"type": "object", "properties": {
                    "title": {"type": "string", "description": "A short title, at most six words."},
                    "task": {"type": "string", "description": "The helper's whole task, self-contained: every detail it needs and which files to write in its folder."}
                }, "required": ["task"]}}
            }, "required": ["jobs"]},
        }),
        json!({
            "name": "hex_team_tell",
            "description": "Give your helpers a new instruction while they work (the person changed what they want, or a helper should change course): it reaches the helper at once, as the person's words would. `to` is a helper's id from hex_team_start, or \"all\".",
            "inputSchema": {"type": "object", "properties": {
                "to": {"type": "string"},
                "text": {"type": "string", "description": "The instruction, self-contained."}
            }, "required": ["to", "text"]},
        }),
        json!({
            "name": "hex_team_status",
            "description": "Your helpers: each one's state, current step, and once it ended its final answer. With wait_seconds (up to 240) it waits until the next helper finishes or asks the person something, so you can wait without polling.",
            "inputSchema": {"type": "object", "properties": {
                "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 240}
            }},
        }),
    ]
}

/// The tool definitions for a quick question or a chat.
pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "hex_start_jobs",
            "description": "Hand the request to the background as one or more jobs, when it is a job rather than a quick question or a small action in the window in front (several steps, many items, other apps or websites, research, documents, files, or more than a minute or two). Hex starts each job with every tool it needs (its own browsers, apps, files) and the job asks the person what it needs. Call it once with all jobs: unrelated requests become separate jobs that run in parallel; parts that build on each other (research, then a sheet, then a message) stay one job, which leads helpers itself. Then end your reply with one or two short, speakable sentences saying you're on it. Never say something runs in the background without calling this.",
            "inputSchema": {"type": "object", "properties": {
                "jobs": {"type": "array", "items": {"type": "object", "properties": {
                    "title": {"type": "string", "description": "A short title, at most six words."},
                    "task": {"type": "string", "description": "The job in one self-contained sentence, as the person asked for it (with every detail they gave)."}
                }, "required": ["task"]}},
                "say": {"type": "string", "description": "The short line to say meanwhile, in the person's language."}
            }, "required": ["jobs"]},
        }),
        json!({
            "name": "hex_remember",
            "description": "Remember one durable fact the person told you, for every later task: their company, website, a colleague's name and email, the tone or language they like. One short fact per call. Never passwords, keys, tokens, card or account numbers.",
            "inputSchema": {"type": "object", "properties": {
                "fact": {"type": "string", "description": "The fact, as one short sentence."}
            }, "required": ["fact"]},
        }),
        json!({
            "name": "hex_schedule",
            "description": "Schedule a job to run by itself later or repeatedly (\"every morning at 8, …\"), in this computer's local time. Each run is a background job doing `task`. Tell the person the schedule and its next runs in your answer.",
            "inputSchema": {"type": "object", "properties": {
                "when": {"type": "string", "description": "A cron line (minute hour day month weekday, e.g. \"0 8 * * 1-5\") or an RRULE (e.g. \"FREQ=WEEKLY;BYDAY=MO;BYHOUR=9\")."},
                "task": {"type": "string", "description": "What to do each time, as the person would say it."},
                "lane": {"type": "string", "enum": ["background", "here"], "description": "Always background in practice."}
            }, "required": ["when", "task"]},
        }),
        json!({
            "name": "hex_schedules",
            "description": "List the scheduled jobs with their ids and next runs.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "hex_compare_images",
            "description": "Compares two images (PNG or JPEG, such as screenshots of a reference page and of your result at the SAME viewport size) at full resolution: the share of differing pixels, whether that is within `threshold` (\"identical\"), a similarity from 0 to 100, the regions that differ most, and with `diff` a PNG showing every differing pixel in red. Use it to prove visual work matches (pixel-identical: threshold 0.5 or less, at desktop and mobile sizes).",
            "inputSchema": {"type": "object", "properties": {
                "a": {"type": "string", "description": "Absolute path of the first image (the reference)."},
                "b": {"type": "string", "description": "Absolute path of the second image."},
                "tolerance": {"type": "integer", "description": "Per-channel difference 0-255 a pixel may have and still match (default 16, for anti-aliasing and compression)."},
                "threshold": {"type": "number", "description": "Percent of differing pixels still called identical (default 0.5)."},
                "diff": {"type": "string", "description": "Absolute path to write the diff PNG to (in your workspace)."}
            }, "required": ["a", "b"]},
        }),
        json!({
            "name": "hex_unschedule",
            "description": "Remove a scheduled job by its id (from hex_schedules).",
            "inputSchema": {"type": "object", "properties": {
                "id": {"type": "string"}
            }, "required": ["id"]},
        }),
    ]
}

/// Runs one tool; its text, or why it failed.
pub fn call(name: &str, arguments: &Value) -> Result<String, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name} needs “{key}”"))
    };
    if role() == Role::Job
        && let Some((_, why)) = NOT_FOR_JOBS.iter().find(|(refused, _)| *refused == name)
    {
        return Err((*why).to_owned());
    }
    match name {
        "hex_team_start" | "hex_team_status" | "hex_team_tell" if role() != Role::Job => {
            Err("Only a background job leads a team; hand the work on with hex_start_jobs.".into())
        }
        "hex_team_start" => {
            let lead = TASK
                .get()
                .ok_or("hex_team_start needs the task this server serves")?;
            let root =
                crate::assistant::agent::team::root().ok_or("Hex has no folder for teams")?;
            let jobs = crate::assistant::agent::handoff::parse(arguments)?.jobs;
            let asked: Vec<String> = jobs.iter().map(|job| job.title.clone()).collect();
            let members = crate::assistant::agent::team::start(&root, lead, jobs)?;
            let lines: Vec<String> = members
                .iter()
                .map(|member| format!("- {} (id {})", member.title, member.id))
                .collect();
            let mut text = format!(
                "Started {} helper(s):\n{}\nFollow them with hex_team_status (wait_seconds waits for the next to finish).",
                members.len(),
                lines.join("\n")
            );
            if let Some(waiting) = not_started(&asked, members.len()) {
                text.push_str(&waiting);
            }
            Ok(text)
        }
        "hex_team_tell" => {
            let lead = TASK
                .get()
                .ok_or("hex_team_tell needs the task this server serves")?;
            let root =
                crate::assistant::agent::team::root().ok_or("Hex has no folder for teams")?;
            let told = crate::assistant::agent::team::tell(
                &root,
                lead,
                vec![crate::assistant::agent::team::Tell {
                    to: text("to")?,
                    text: text("text")?,
                }],
            )?;
            if told.is_empty() {
                return Err("No helper was told: none of yours has that id, or it waits for the person's answer to its question (only they answer it; hex_team_status shows each one).".into());
            }
            let names: Vec<&str> = told.iter().map(|member| member.title.as_str()).collect();
            Ok(format!("Told {}: {}.", told.len(), names.join(", ")))
        }
        "hex_team_status" => {
            let lead = TASK
                .get()
                .ok_or("hex_team_status needs the task this server serves")?;
            let root =
                crate::assistant::agent::team::root().ok_or("Hex has no folder for teams")?;
            let wait = arguments
                .get("wait_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let members = crate::assistant::agent::team::wait_for_change(
                &root,
                lead,
                std::time::Duration::from_secs(wait),
            );
            Ok(crate::assistant::agent::team::report(&members))
        }
        "hex_start_jobs" => {
            let task = TASK
                .get()
                .ok_or("hex_start_jobs needs the task this server serves")?;
            let handoff = crate::assistant::agent::handoff::parse(arguments)?;
            let titles: Vec<String> = handoff.jobs.iter().map(|job| job.title.clone()).collect();
            let count = crate::assistant::agent::handoff::record(task, handoff)?;
            Ok(format!(
                "Hex starts {count} background job(s) as soon as you end your reply: {}. Now end with one or two short sentences saying you're on it; don't do the work yourself.",
                titles.join("; ")
            ))
        }
        "hex_compare_images" => {
            let defaults = images::Options::default();
            let options = images::Options {
                tolerance: arguments
                    .get("tolerance")
                    .and_then(Value::as_u64)
                    .map_or(defaults.tolerance, |value| value.min(255) as u8),
                threshold: arguments
                    .get("threshold")
                    .and_then(Value::as_f64)
                    .map_or(defaults.threshold, |value| value.clamp(0.0, 100.0)),
            };
            let diff = arguments.get("diff").and_then(Value::as_str);
            images::compare(&text("a")?, &text("b")?, options, diff)
        }
        "hex_remember" => {
            if !memory_on() {
                return Err("The person turned memory off; nothing is remembered.".into());
            }
            memory::remember(&text("fact")?)
                .map(|line| format!("Remembered: {line}"))
                .map_err(|error| format!("{error:#}"))
        }
        "hex_schedule" => {
            let lane = match arguments.get("lane").and_then(Value::as_str) {
                Some("here") => Lane::Here,
                _ => Lane::Background,
            };
            let schedule = schedules::add_by_agent(&text("when")?, &text("task")?, lane)
                .map_err(|error| format!("{error:#}"))?;
            let next = schedules::next_runs(&schedule.when, 3)
                .unwrap_or_default()
                .into_iter()
                .map(schedules::describe)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "Scheduled {} ({}): “{}”. Next runs: {next}.",
                schedule.id, schedule.when, schedule.task
            ))
        }
        "hex_schedules" => {
            let list = schedules::list();
            if list.is_empty() {
                return Ok("No scheduled jobs.".into());
            }
            Ok(list
                .iter()
                .map(|schedule| {
                    let next = schedule
                        .next_due()
                        .map_or_else(|| "never".into(), schedules::describe);
                    format!(
                        "{} [{}] {}: “{}”, next {next}",
                        schedule.id,
                        if schedule.enabled { "on" } else { "off" },
                        schedule.when,
                        schedule.task
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "hex_unschedule" => {
            let id = text("id")?;
            if schedules::remove(&id) {
                Ok(format!("Removed the schedule {id}."))
            } else {
                Err(format!("There is no schedule {id}."))
            }
        }
        other => Err(format!("There is no tool “{other}”.")),
    }
}

fn memory_on() -> bool {
    crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.memory)
        .or_else(|| {
            crate::app_settings::AppSettings::read()
                .ok()
                .map(|settings| settings.assistant.memory)
        })
        .unwrap_or(true)
}

/// Serves the tools on stdio until the agent disconnects.
pub fn serve(task: &str, role: Role) -> Result<()> {
    let _ = TASK.set(task.to_owned());
    let _ = JOB.set(role == Role::Job);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(reply) = answer(&message) {
            let mut text = serde_json::to_vec(&reply)?;
            text.push(b'\n');
            stdout.write_all(&text)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The reply to one JSON-RPC message; `None` for a notification.
pub fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params
                .get("protocolVersion")
                .cloned()
                .unwrap_or_else(|| json!("2025-06-18")),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "hex", "version": crate::VERSION},
        }),
        "tools/list" => json!({"tools": definitions_for(role())}),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call(name, &arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            json!({"content": [{"type": "text", "text": text}], "isError": is_error})
        }
        "ping" => json!({}),
        other => {
            return Some(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {other}")}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speaks_mcp_and_refuses_unknown_tools_and_secrets() {
        let init = answer(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}}))
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "hex");
        assert!(
            answer(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none()
        );
        let list = answer(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "hex_start_jobs",
                "hex_remember",
                "hex_schedule",
                "hex_schedules",
                "hex_compare_images",
                "hex_unschedule"
            ]
        );
        let secret = answer(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "hex_remember", "arguments": {"fact": "My password is hunter2"}}}))
        .unwrap();
        assert_eq!(secret["result"]["isError"], true);
        let bad = call("hex_schedule", &json!({"when": "whenever", "task": "x"})).unwrap_err();
        assert!(bad.contains("isn't a schedule"), "{bad}");
        assert!(call("hex_nothing", &json!({})).is_err());
        assert!(
            call("hex_team_status", &json!({}))
                .unwrap_err()
                .contains("Only a background job")
        );
    }

    #[test]
    fn a_job_leads_a_team_and_can_t_change_what_outlives_it() {
        let names: Vec<String> = definitions_for(Role::Job)
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(
            names,
            [
                "hex_schedules",
                "hex_compare_images",
                "hex_team_start",
                "hex_team_tell",
                "hex_team_status"
            ]
        );
        assert_eq!(definitions_for(Role::Turn), definitions());
        assert_eq!(Role::Job.arg(), "job");
    }

    #[test]
    fn a_lead_hears_which_helpers_wait_for_room() {
        let asked: Vec<String> = ["A", "B", "C"].map(str::to_owned).to_vec();
        assert_eq!(not_started(&asked, 3), None);
        assert_eq!(not_started(&asked, 5), None);
        let waiting = not_started(&asked, 1).unwrap();
        assert!(waiting.contains("B; C"), "{waiting}");
    }
}
