//! What connects one request to the others: a short digest of the tasks
//! running and the conversations just finished, for every new
//! conversation's first prompt ("do the same for the other mail", "and now
//! the second one"), and a cheap matcher that sends an instruction naming
//! one running task ("bei der Mail-Aufgabe …", "stop the invoice task") to
//! that task as steering.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::assistant::store;
use crate::assistant::types::{ConversationId, ConversationSummary, Origin, Outcome, TaskStatus};

/// The digest's budget: about 1,200 tokens.
const MAX_CHARS: usize = 4800;
/// How many finished conversations it lists.
const RECENT: usize = 10;

/// A task's short id in the digest, as `hex_tell_job` and `hex_job_result`
/// take it (the conversation id's first eight characters).
pub fn short_id(conversation: &str) -> &str {
    conversation.get(..8).unwrap_or(conversation)
}

fn state(state: crate::assistant::TaskState) -> &'static str {
    use crate::assistant::TaskState;
    match state {
        TaskState::Queued => "queued",
        TaskState::Running => "working",
        TaskState::Paused => "the person took it over",
        TaskState::Waiting { .. } => "waiting for a usage limit",
        TaskState::NeedsInput => "waits for the person's answer",
    }
}

/// What else is going on, for a new conversation's first prompt; `None`
/// when nothing is. A person's own `turn` also learns the tools that read
/// and steer the tasks (a job has none of them: `tools::Role`).
pub fn digest(
    running: &[TaskStatus],
    recent: &[ConversationSummary],
    now_ms: u64,
    turn: bool,
) -> Option<String> {
    let mut lines = Vec::new();
    if !running.is_empty() {
        lines.push("Running now:".to_owned());
        for (index, task) in running.iter().enumerate() {
            let mut line = format!(
                "{}. [{}] “{}” ({}, {})",
                index + 1,
                short_id(&task.conversation),
                task.title,
                lane(task.origin),
                state(task.state)
            );
            if let Some(step) = &task.step {
                line.push_str(&format!(", now: {}", store::shorten(step, 60)));
            }
            let apps: Vec<&str> = task
                .windows
                .iter()
                .map(|window| window.app.as_str())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            if !apps.is_empty() {
                let mut apps = apps;
                apps.sort_unstable();
                line.push_str(&format!(", in {}", apps.join(", ")));
            }
            lines.push(line);
        }
    }
    let busy: HashSet<&str> = running
        .iter()
        .map(|task| task.conversation.as_str())
        .collect();
    let finished: Vec<&ConversationSummary> = recent
        .iter()
        .filter(|summary| !busy.contains(summary.id.as_str()))
        .filter(|summary| summary.outcome != Outcome::Running)
        .take(RECENT)
        .collect();
    if !finished.is_empty() {
        lines.push("Finished recently (newest first):".to_owned());
        for (index, summary) in finished.iter().enumerate() {
            let preview = store::shorten(&summary.preview, 120);
            lines.push(format!(
                "{}. [{}] “{}” — {} {}: {preview}",
                index + 1,
                short_id(&summary.id),
                summary.title,
                outcome(summary.outcome),
                ago(now_ms.saturating_sub(summary.updated_ms)),
            ));
        }
    }
    if lines.is_empty() {
        return None;
    }
    let mut text = String::from(
        "Everything you did for the person lately, across tasks: one assistant, one stream. The person may refer to any of it (\u{201c}the Tokio one\u{201d}, \u{201c}that task\u{201d}, \u{201c}what did it find?\u{201d}).",
    );
    text.push_str(if turn {
        " Answer from here, read a task's full result with hex_job_result([id]), and change a running job with hex_tell_job([id], text); start new work with hex_start_jobs.\n"
    } else {
        " Use what is here for your job.\n"
    });
    for line in lines {
        if text.chars().count() + line.chars().count() + 1 > MAX_CHARS {
            break;
        }
        text.push_str(&line);
        text.push('\n');
    }
    Some(text.trim_end().to_owned())
}

fn lane(origin: Origin) -> &'static str {
    match origin {
        Origin::ActionShortcut => "here",
        Origin::Background => "background",
        Origin::Chat => "chat",
    }
}

fn outcome(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Answered => "answered",
        Outcome::Failed => "failed",
        Outcome::Cancelled => "stopped",
        Outcome::Interrupted => "interrupted",
        Outcome::Running => "running",
    }
}

fn ago(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// The running task `text` names, when it names exactly one: it speaks of a
/// task ("Aufgabe", "task", "bei der …", "stop …") and shares a word with
/// one task's title or apps more than with any other.
pub fn route(text: &str, running: &[TaskStatus]) -> Option<ConversationId> {
    // A question about a task ("Was hat die Tokio-Aufgabe gefunden?") is
    // answered from the stream (`digest`, `hex_job_result`), not steered.
    if running.is_empty() || !cue().is_match(text) || asks_about(text) {
        return None;
    }
    let asked = words(text);
    let mut best: Option<(&TaskStatus, usize)> = None;
    let mut tie = false;
    for task in running {
        let mut names = words(&task.title);
        for window in &task.windows {
            names.extend(words(&window.app));
        }
        let score = asked.intersection(&names).count();
        if score == 0 {
            continue;
        }
        match best {
            Some((_, top)) if score < top => {}
            Some((_, top)) if score == top => tie = true,
            _ => {
                best = Some((task, score));
                tie = false;
            }
        }
    }
    let (task, _) = best?;
    (!tie).then(|| task.conversation.clone())
}

/// Words that tell an instruction is about a task.
/// Whether `text` asks about something rather than tells a task to do it.
fn asks_about(text: &str) -> bool {
    let text = text.trim();
    let first = text
        .split(|c: char| !c.is_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or_default()
        .to_lowercase();
    text.ends_with('?')
        && [
            "was", "wie", "wo", "wann", "welche", "welcher", "welches", "warum", "hat", "ist",
            "sind", "gibt", "what", "how", "where", "when", "which", "why", "did", "has", "is",
            "are",
        ]
        .contains(&first.as_str())
}

fn cue() -> &'static Regex {
    static CUE: OnceLock<Regex> = OnceLock::new();
    CUE.get_or_init(|| {
        Regex::new(
            r"(?i)(aufgabe|task|auftrag)\b|\bbei (der|dem|den)\b|\bbeim\b|\bstopp?\b|\bhalt\b|\babbrechen\b|\bcancel\b|\bfor the\b|\bin the\b.*\b(one|job)\b",
        )
        .expect("valid pattern")
    })
}

/// `text`'s meaningful words, lowercased, without filler and task words.
pub(super) fn words(text: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "aufgabe",
        "aufgaben",
        "task",
        "tasks",
        "auftrag",
        "der",
        "die",
        "das",
        "den",
        "dem",
        "des",
        "bei",
        "beim",
        "und",
        "oder",
        "mit",
        "für",
        "von",
        "auf",
        "aus",
        "ein",
        "eine",
        "einen",
        "einer",
        "noch",
        "auch",
        "jetzt",
        "bitte",
        "mach",
        "mache",
        "machen",
        "nimm",
        "stopp",
        "stop",
        "halt",
        "abbrechen",
        "lieber",
        "doch",
        "nicht",
        "the",
        "and",
        "for",
        "with",
        "that",
        "this",
        "please",
        "now",
        "one",
        "job",
        "cancel",
        "also",
        "then",
        "into",
        "ist",
        "sie",
        "ihr",
        "ihre",
        "ihn",
        "was",
        "wie",
        "you",
        "your",
        "can",
        "kannst",
        "du",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() >= 3 && !STOP.contains(&word.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::types::{TaskState, TaskWindow};

    fn task(id: &str, title: &str, app: &str) -> TaskStatus {
        TaskStatus {
            conversation: id.into(),
            origin: Origin::Background,
            title: title.into(),
            state: TaskState::Running,
            step: Some("Typing 12 characters into “To”".into()),
            windows: vec![TaskWindow {
                app: app.into(),
                window_id: None,
            }],
            colour: 0,
        }
    }

    fn summary(id: &str, title: &str, outcome: Outcome, updated_ms: u64) -> ConversationSummary {
        ConversationSummary {
            id: id.into(),
            title: title.into(),
            updated_ms,
            origin: Origin::ActionShortcut,
            preview: format!("{title} done"),
            outcome,
        }
    }

    #[test]
    fn an_instruction_naming_one_task_goes_to_it() {
        let running = [
            task("mail", "Antworte Anna auf ihre Mail", "Mail"),
            task(
                "invoice",
                "Lade die Rechnung von Hetzner herunter",
                "Safari",
            ),
        ];
        assert_eq!(
            route("bei der Mail-Aufgabe nimm lieber Freitag", &running).as_deref(),
            Some("mail")
        );
        assert_eq!(
            route("stopp den Rechnung-Task", &running).as_deref(),
            Some("invoice")
        );
        // No word about a task: a new task, with the digest.
        assert_eq!(route("mach das auch für die andere Mail", &running), None);
        // Named twice over: ambiguous, a new task.
        let twins = [
            task("a", "Mail an Anna", "Mail"),
            task("b", "Mail an Bob", "Mail"),
        ];
        assert_eq!(route("bei der Mail-Aufgabe warte noch", &twins), None);
        assert_eq!(route("bei der Aufgabe warte", &[]), None);
    }

    #[test]
    fn the_digest_lists_running_tasks_and_recent_outcomes_compactly() {
        let now = 10 * 60 * 60_000;
        let running = [task("mail", "Antworte Anna", "Mail")];
        let recent = [
            summary("mail", "Antworte Anna", Outcome::Running, now),
            summary(
                "q1",
                "Wie spät ist es in Tokio?",
                Outcome::Answered,
                now - 5 * 60_000,
            ),
            summary(
                "q2",
                "Rechnung suchen",
                Outcome::Failed,
                now - 2 * 60 * 60_000,
            ),
        ];
        let digest = digest(&running, &recent, now, true).unwrap();
        // A job has none of the tools that read or steer tasks.
        let job = super::digest(&running, &recent, now, false).unwrap();
        assert!(!job.contains("hex_"), "{job}");
        assert_eq!(
            digest,
            "Everything you did for the person lately, across tasks: one assistant, one stream. The person may refer to any of it (\u{201c}the Tokio one\u{201d}, \u{201c}that task\u{201d}, \u{201c}what did it find?\u{201d}). Answer from here, read a task's full result with hex_job_result([id]), and change a running job with hex_tell_job([id], text); start new work with hex_start_jobs.\n\
             Running now:\n\
             1. [mail] “Antworte Anna” (background, working), now: Typing 12 characters into “To”, in Mail\n\
             Finished recently (newest first):\n\
             1. [q1] “Wie spät ist es in Tokio?” — answered 5 min ago: Wie spät ist es in Tokio? done\n\
             2. [q2] “Rechnung suchen” — failed 2 h ago: Rechnung suchen done"
        );
        assert_eq!(super::digest(&[], &[], now, true), None);
        let many: Vec<_> = (0..40)
            .map(|i| {
                summary(
                    &format!("c{i}"),
                    &"lang ".repeat(30),
                    Outcome::Answered,
                    now,
                )
            })
            .collect();
        let long = super::digest(&[], &many, now, true).unwrap();
        assert!(long.chars().count() <= MAX_CHARS);
        assert_eq!(long.matches(" — answered").count(), RECENT);
    }
}
