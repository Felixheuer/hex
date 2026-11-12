//! What every task did, call by call, for the person to read afterwards:
//! hours of unattended work leave a trail of every shell command, file
//! written, click, page opened, message sent, and helper started, not only
//! the agent's own account of it.
//!
//! Every backend reports each tool call its agent makes
//! (`provider::Report::Tool`); the runner appends it to
//! `<assistant>/audit/<conversation>.jsonl`, one JSON object a line, as it
//! happens. Secrets in the arguments (anything named like a password, token,
//! key, or card number) are blanked, and long values cut. A job's check
//! appears marked as such. `hex assistant audit <conversation>` prints it,
//! and a job that ends leaves it in its folder as `Hex-Protokoll.txt`.

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The longest value kept of an argument.
const VALUE_KEPT: usize = 400;
/// The longest arguments kept of one call.
const INPUT_KEPT: usize = 1_500;
/// A log larger than this is cut to its newer half when written to.
const LOG_MAX: u64 = 8 * 1024 * 1024;

/// One call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix ms.
    pub at_ms: u64,
    /// The tool, as the agent named it (`Bash`, `mcp__hex-desktop__desktop_click`).
    pub tool: String,
    /// Its arguments, compact and with secrets blanked.
    pub input: String,
    /// Made by the job's check (which only looks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check: bool,
}

fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dir = crate::assistant::store::assistant_dir().ok()?.join("audit");
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

fn path(conversation: &str) -> Option<PathBuf> {
    let safe = !conversation.is_empty()
        && conversation
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    safe.then(|| dir().map(|dir| dir.join(format!("{conversation}.jsonl"))))?
}

/// Appends `entry` to `conversation`'s log.
pub fn record(conversation: &str, entry: &Entry) {
    let Some(path) = path(conversation) else {
        return;
    };
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_MAX) {
        shrink(&path);
    }
    let Ok(mut line) = serde_json::to_vec(entry) else {
        return;
    };
    line.push(b'\n');
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(&line));
    if let Err(error) = written {
        tracing::debug!(%error, "couldn't add to the audit log");
    }
}

/// Keeps the newer half of an oversized log.
fn shrink(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let kept = lines[lines.len() / 2..].join("\n");
    let _ = crate::durable::write(path, format!("{kept}\n").as_bytes());
}

/// `conversation`'s log, oldest first.
pub fn read(conversation: &str) -> Vec<Entry> {
    let Some(path) = path(conversation) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// What `hex assistant audit` prints.
pub fn report(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "Nothing recorded for this conversation.\n".into();
    }
    let mut out = String::new();
    for entry in entries {
        let clock = crate::assistant::schedules::describe(entry.at_ms);
        out.push_str(&format!(
            "{clock}  {}{}  {}\n",
            if entry.check { "[check] " } else { "" },
            short_tool(&entry.tool),
            entry.input
        ));
    }
    out
}

/// The log's file in a job's folder.
pub const IN_FOLDER: &str = "Hex-Protokoll.txt";

/// Writes `conversation`'s log as a timeline into its job's `folder`
/// ([`IN_FOLDER`]), when there is one.
pub fn keep_in(conversation: &str, folder: &std::path::Path) {
    let entries = read(conversation);
    if entries.is_empty() {
        return;
    }
    if let Err(error) = std::fs::write(folder.join(IN_FOLDER), report(&entries)) {
        tracing::debug!(%error, "couldn't keep the log in the job's folder");
    }
}

/// `desktop_click` for `mcp__hex-desktop__desktop_click`.
fn short_tool(tool: &str) -> &str {
    tool.rsplit("__").next().unwrap_or(tool)
}

/// A tool call's arguments as one line: compact JSON, secrets blanked,
/// long values cut.
pub fn compact(input: &Value) -> String {
    let mut value = input.clone();
    redact(&mut value, false);
    let text = value.to_string();
    if text.chars().count() > INPUT_KEPT {
        let cut: String = text.chars().take(INPUT_KEPT).collect();
        format!("{cut}…")
    } else {
        text
    }
}

/// Whether an argument named `key` holds a secret.
fn secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "api_key",
        "authorization",
        "cookie",
        "otp",
        "pin",
        "cvv",
        "cvc",
        "card",
    ]
    .iter()
    .any(|word| key.contains(word))
        || key == "key"
}

fn redact(value: &mut Value, hidden: bool) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                redact(inner, hidden || secret(key));
            }
        }
        Value::Array(items) => {
            for inner in items {
                redact(inner, hidden);
            }
        }
        Value::String(text) if hidden => *text = "•••".into(),
        Value::String(text) if text.chars().count() > VALUE_KEPT => {
            let cut: String = text.chars().take(VALUE_KEPT).collect();
            *text = format!("{cut}…");
        }
        Value::Number(_) if hidden => *value = Value::String("•••".into()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secrets_are_blanked_and_long_values_cut() {
        let input = json!({
            "command": "git push origin main",
            "password": "hunter2",
            "headers": {"Authorization": "Bearer abc", "accept": "json"},
            "fields": [{"name": "email", "value": "max@example.com"}],
            "card_number": 4111111111111111u64,
            "text": "x".repeat(1000),
        });
        let line = compact(&input);
        assert!(line.contains("git push origin main"));
        assert!(!line.contains("hunter2") && !line.contains("Bearer"));
        assert!(!line.contains("4111"));
        assert!(
            line.contains("max@example.com"),
            "plain values stay: {line}"
        );
        assert!(line.chars().count() <= INPUT_KEPT + 1);
        assert!(!line.contains(&"x".repeat(VALUE_KEPT + 1)));
    }

    #[test]
    fn the_report_reads_as_a_timeline() {
        let entries = [
            Entry {
                at_ms: 1_700_000_000_000,
                tool: "Bash".into(),
                input: r#"{"command":"npm test"}"#.into(),
                check: false,
            },
            Entry {
                at_ms: 1_700_000_060_000,
                tool: "mcp__hex-desktop__desktop_snapshot".into(),
                input: r#"{"app":"Excel"}"#.into(),
                check: true,
            },
        ];
        let text = report(&entries);
        assert!(text.contains("Bash  {\"command\":\"npm test\"}"), "{text}");
        assert!(text.contains("[check] desktop_snapshot"), "{text}");
        assert!(report(&[]).contains("Nothing recorded"));
        assert!(path("../etc").is_none());
    }
}
