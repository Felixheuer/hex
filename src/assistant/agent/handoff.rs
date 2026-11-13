//! Handing a request to the background, deterministically.
//!
//! A quick question or a chat that turns out to be a job used to say so
//! only with a text marker (`HEX_JOB:`, `markers`); an agent that answered
//! in prose instead („Das läuft als Hintergrundauftrag …“) started nothing.
//! Now the agent calls Agent-Desk's `hex_start_jobs` tool (`tools`), which leaves
//! the jobs in a small file per conversation; when the run ends, the runner
//! takes them ([`take`]) and starts them. The marker stays as a fallback,
//! and an answer that *claims* it went to the background without either
//! still starts one (`route::claims_background`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One job the agent hands on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Job {
    pub title: String,
    pub task: String,
}

/// What `hex_start_jobs` left for a conversation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Handoff {
    pub jobs: Vec<Job>,
    /// The short line said meanwhile, if the agent gave one.
    #[serde(default)]
    pub say: Option<String>,
    /// Instructions for running jobs (`hex_tell_job`): (job id or title,
    /// text), passed on when the run ends.
    #[serde(default)]
    pub tells: Vec<(String, String)>,
}

fn dir() -> Option<PathBuf> {
    let dir = crate::assistant::store::assistant_dir()
        .ok()?
        .join("handoff");
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

fn file(conversation: &str) -> Option<PathBuf> {
    let safe = conversation
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (safe && !conversation.is_empty())
        .then(|| dir().map(|dir| dir.join(format!("{conversation}.json"))))?
}

/// Records `handoff` for `conversation` (the tools server's side); jobs
/// already recorded in this run are kept.
pub fn record(conversation: &str, handoff: Handoff) -> Result<usize, String> {
    let path = file(conversation).ok_or("this task has no id Agent-Desk can use")?;
    let mut all: Handoff = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    all.jobs.extend(handoff.jobs);
    all.tells.extend(handoff.tells);
    if handoff.say.is_some() {
        all.say = handoff.say;
    }
    let bytes = serde_json::to_vec(&all).map_err(|error| error.to_string())?;
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(all.jobs.len())
}

/// Takes what was handed on in `conversation`'s run, if anything (the
/// runner's side; the file goes).
pub fn take(conversation: &str) -> Option<Handoff> {
    let path = file(conversation)?;
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_slice::<Handoff>(&bytes)
        .ok()
        .filter(|handoff| !handoff.jobs.is_empty() || !handoff.tells.is_empty())
}

/// Forgets a hand-off a run left but didn't end with (a stopped run).
pub fn clear(conversation: &str) {
    if let Some(path) = file(conversation) {
        let _ = std::fs::remove_file(path);
    }
}

/// Reads `hex_start_jobs`' arguments: `jobs` (each `title` and `task`) and
/// an optional `say`.
pub fn parse(arguments: &serde_json::Value) -> Result<Handoff, String> {
    let jobs = arguments
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .ok_or("hex_start_jobs needs “jobs”: a list of {title, task}")?;
    let jobs: Vec<Job> = jobs
        .iter()
        .filter_map(|job| {
            let task = job.get("task")?.as_str()?.trim();
            if task.is_empty() {
                return None;
            }
            let title = job
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or(task);
            Some(Job {
                title: crate::assistant::store::shorten(title, 60),
                task: task.to_owned(),
            })
        })
        .collect();
    if jobs.is_empty() {
        return Err("hex_start_jobs needs at least one job with a “task”".into());
    }
    let say = arguments
        .get("say")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|say| !say.is_empty())
        .map(str::to_owned);
    Ok(Handoff {
        jobs,
        say,
        tells: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_tool_s_jobs_are_read_and_titled() {
        let handoff = parse(&json!({
            "jobs": [
                {"title": "Werbevideo", "task": "Mach ein Werbevideo über example.com"},
                {"task": "Recherchiere drei Wettbewerber"},
                {"title": "leer", "task": "  "}
            ],
            "say": "Mach ich, zwei Aufträge laufen."
        }))
        .unwrap();
        assert_eq!(handoff.jobs.len(), 2);
        assert_eq!(handoff.jobs[0].title, "Werbevideo");
        assert_eq!(handoff.jobs[1].title, "Recherchiere drei Wettbewerber");
        assert_eq!(
            handoff.say.as_deref(),
            Some("Mach ich, zwei Aufträge laufen.")
        );
        assert!(parse(&json!({"jobs": []})).is_err());
        assert!(parse(&json!({})).is_err());
    }
}
