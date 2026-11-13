//! What the jobs are doing, told only when the person asks ("wie weit ist
//! der Shop?", "was machen die Aufgaben?", "status"): jobs never announce
//! their steps on their own. The answer comes from each job's plan
//! (`plan.md`, ticked steps of all), how long it has run, and its last
//! independent check (`agent::goal`, `.hex-check.md` in its workspace):
//! short, and spoken like any answer, with the HUD's bubble showing it.

use std::sync::Arc;

use super::{JobKind, Origin, Request, Runner, TaskState};
use crate::assistant::agent::goal;
use crate::assistant::lines::tr_named_in;
use crate::assistant::types::{ConversationId, Event};
use crate::desktop::i18n::tr_in;

/// One job, as the status tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub title: String,
    pub state: TaskState,
    pub elapsed_ms: u64,
    /// Ticked and all steps of its plan.
    pub done: usize,
    pub steps: usize,
    /// What its last check found missing, if it found anything.
    pub gaps: Option<String>,
    /// Its helpers (`agent::team`), ended and in all, when it leads some.
    pub helpers: Option<(usize, usize)>,
}

/// `lead`'s helpers, ended and in all, from the team's records; `None`
/// when it leads none.
fn helpers_of(lead: &str) -> Option<(usize, usize)> {
    let root = crate::assistant::agent::team::root()?;
    let members = crate::assistant::agent::team::members(&root, lead);
    let ended = members.iter().filter(|member| member.state.ended()).count();
    (!members.is_empty()).then_some((ended, members.len()))
}

/// Whether `text` asks how the jobs are doing. A plain "how far" counts
/// only with a job's word or name in it, so "wie weit ist es nach Berlin"
/// stays a question for the assistant.
pub fn asks_for_status(text: &str, titles: &[String]) -> bool {
    let lower = text.to_lowercase();
    let lower = lower.trim().trim_end_matches(['?', '.', '!']);
    const ALWAYS: &[&str] = &[
        "status",
        "fortschritt",
        "was machen die aufgaben",
        "was machen meine aufgaben",
        "welche aufgaben laufen",
        "wie läuft es mit",
        "wie laufen die aufgaben",
        "progress",
        "what are the tasks doing",
        "what are my tasks doing",
        "what's running",
        "what is running",
        "how are the tasks",
        "how are my tasks",
    ];
    // A status question is short; a longer request that mentions progress
    // ("show the upload progress in the sheet") is a request.
    if lower.split_whitespace().count() > 9 {
        return false;
    }
    if ALWAYS.iter().any(|phrase| lower.contains(phrase)) {
        return true;
    }
    let how_far = [
        "wie weit",
        "how far",
        "wie steht",
        "how is the",
        "how's the",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    how_far
        && (["aufgabe", "task", "job", "auftrag"]
            .iter()
            .any(|word| lower.contains(word))
            || names_one(lower, titles).is_some())
}

/// Whether `text`, said inside a job's own conversation, asks how it is
/// doing: there "wie weit bist du?" needs no job's name.
pub fn asks_how_it_goes(text: &str) -> bool {
    let lower = text.to_lowercase();
    let lower = lower.trim().trim_end_matches(['?', '.', '!']).trim();
    if lower.is_empty() || lower.split_whitespace().count() > 8 {
        return false;
    }
    const ASKS: &[&str] = &[
        "wie weit",
        "wie lange noch",
        "wie lange dauert",
        "was machst du",
        "was tust du",
        "woran arbeitest du",
        "bist du fertig",
        "bist du schon fertig",
        "status",
        "fortschritt",
        "wie sieht's aus",
        "wie siehts aus",
        "wie sieht es aus",
        "wie läuft",
        "how far",
        "how long",
        "what are you doing",
        "what are you working on",
        "are you done",
        "done yet",
        "progress",
        "how is it going",
        "how's it going",
    ];
    ASKS.iter().any(|phrase| lower.contains(phrase))
}

/// The job whose title `lower` names (a word of four letters or more).
fn names_one(lower: &str, titles: &[String]) -> Option<usize> {
    titles.iter().position(|title| {
        title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .any(|word| lower.contains(word))
    })
}

/// The spoken status of `jobs`, or of the one `asked` names, in
/// `language` (`lines::language` of what was asked; `None` is the
/// interface's).
pub fn line(jobs: &[Job], asked: &str, language: Option<&str>) -> String {
    let titles: Vec<String> = jobs.iter().map(|job| job.title.clone()).collect();
    let lower = asked.to_lowercase();
    let picked: Vec<&Job> = match names_one(&lower, &titles) {
        Some(index) => vec![&jobs[index]],
        None => jobs.iter().collect(),
    };
    if picked.is_empty() {
        return tr_in(language, "No task is running.").into();
    }
    picked
        .iter()
        .map(|job| one(job, language))
        .collect::<Vec<_>>()
        .join(" ")
}

fn one(job: &Job, language: Option<&str>) -> String {
    let elapsed = duration(job.elapsed_ms, language);
    let template = match job.state {
        TaskState::NeedsInput => "{title}: waiting for your answer",
        TaskState::Waiting { .. } => "{title}: waiting out a usage limit, running for {elapsed}",
        TaskState::Paused => "{title}: paused, you took over",
        TaskState::Queued => "{title}: waiting for a free slot",
        TaskState::Running => "{title}: running for {elapsed}",
    };
    let mut line = tr_named_in(
        language,
        template,
        &[("title", &job.title), ("elapsed", &elapsed)],
    );
    if job.steps > 0 {
        line.push_str(&tr_named_in(
            language,
            ", {done} of {steps} steps done",
            &[
                ("done", &job.done.to_string()),
                ("steps", &job.steps.to_string()),
            ],
        ));
    }
    if let Some((ended, all)) = job.helpers {
        line.push_str(&tr_named_in(
            language,
            ", {ended} of {all} helpers done",
            &[("ended", &ended.to_string()), ("all", &all.to_string())],
        ));
    }
    if let Some(gaps) = job.gaps.as_deref().and_then(|gaps| gaps.lines().next()) {
        let gaps = gaps.trim().trim_start_matches(['-', '*', ' ']);
        line.push_str(&tr_named_in(
            language,
            ". Still open: {gaps}",
            &[("gaps", gaps)],
        ));
    }
    line.push('.');
    line
}

/// "3 Stunden", "2 days", "12 minutes".
fn duration(ms: u64, language: Option<&str>) -> String {
    let minutes = ms / 60_000;
    let (n, unit) = if minutes >= 2 * 24 * 60 {
        (minutes / (24 * 60), "{n} days")
    } else if minutes >= 120 {
        (minutes / 60, "{n} hours")
    } else {
        (minutes.max(1), "{n} minutes")
    };
    if n == 1 {
        return tr_in(language, "a minute").into();
    }
    tr_named_in(language, unit, &[("n", &n.to_string())])
}

/// What was said in a job's conversation while the job is there.
enum InJob {
    /// It asks how the job is doing: that job, and its folder.
    Status(Job, Option<std::path::PathBuf>),
    /// Said while the job waits for its turn: added to its request.
    Added,
}

impl Runner {
    /// A message in the conversation of a job that is queued or works:
    /// "wie weit bist du?" is answered at once from the job's plan, in that
    /// conversation, without stopping the job; anything else said while the
    /// job still waits for its turn is added to its request (a working job
    /// is steered as usual: `None`). Both are written into the conversation
    /// where they were said (`Hooks::note`), with Agent-Desk's reply.
    pub(super) fn in_job(self: &Arc<Self>, request: &Request) -> Option<ConversationId> {
        let conversation = request.conversation.clone()?;
        let now = self.now();
        let found = {
            let mut state = self.state();
            let task = state.tasks.iter_mut().find(|task| {
                task.status.conversation == conversation
                    && task.status.state != TaskState::NeedsInput
                    && (task.workspace.is_some() || matches!(task.kind, JobKind::Job { .. }))
            })?;
            if asks_how_it_goes(&request.text)
                || asks_for_status(&request.text, std::slice::from_ref(&task.status.title))
            {
                InJob::Status(
                    Job {
                        title: task.status.title.clone(),
                        state: task.status.state,
                        elapsed_ms: now.saturating_sub(task.started_ms),
                        done: task.plan.iter().filter(|step| step.done).count(),
                        steps: task.plan.len(),
                        gaps: None,
                        helpers: None,
                    },
                    task.workspace.clone(),
                )
            } else if !task.running {
                task.request.text = format!(
                    "{}\n\n{}",
                    task.request.text.trim_end(),
                    request.text.trim()
                );
                InJob::Added
            } else {
                return None;
            }
        };
        let language = crate::assistant::lines::language(&request.text);
        self.note(
            &conversation,
            crate::assistant::types::Role::User,
            &request.text,
        );
        let reply = match found {
            InJob::Status(mut job, workspace) => {
                job.gaps = workspace
                    .and_then(|workspace| {
                        std::fs::read_to_string(workspace.join(goal::CHECK_FILE)).ok()
                    })
                    .filter(|check| check.trim() != "verified");
                job.helpers = helpers_of(&conversation);
                line(std::slice::from_ref(&job), &request.text, language)
            }
            InJob::Added => tr_in(language, "Noted, I'll take that in when I start.").to_owned(),
        };
        self.note(
            &conversation,
            crate::assistant::types::Role::Assistant,
            &reply,
        );
        self.emit(&Event::ConversationsChanged);
        if request.origin != Origin::Chat {
            self.voice.announce(
                self,
                super::voice::Priority::Answer,
                &conversation,
                "",
                &reply,
            );
        }
        Some(conversation)
    }

    /// Writes one text message into `conversation` beside its task.
    pub(super) fn note(&self, conversation: &str, role: crate::assistant::types::Role, text: &str) {
        (self.hooks.note)(
            conversation,
            crate::assistant::types::Message {
                role,
                parts: vec![crate::assistant::types::Part::Text {
                    text: text.trim().to_owned(),
                }],
                at_ms: crate::assistant::store::now_ms(),
            },
        );
    }

    /// Answers a hold that asks how the jobs are doing (`asks_for_status`),
    /// spoken and shown in the bubble; `None` when it asks something else.
    pub(super) fn status_reply(self: &Arc<Self>, request: &Request) -> Option<ConversationId> {
        if request.origin != Origin::ActionShortcut {
            return None;
        }
        let now = self.now();
        // A team answers as one: its helpers are told in its lead's line.
        let (mut jobs, found): (Vec<Job>, Vec<(std::path::PathBuf, ConversationId)>) = {
            let state = self.state();
            state
                .tasks
                .iter()
                .filter(|task| task.parent.is_none())
                .filter_map(|task| {
                    let workspace = task.workspace.clone()?;
                    Some((
                        Job {
                            title: task.status.title.clone(),
                            state: task.status.state,
                            elapsed_ms: now.saturating_sub(task.started_ms),
                            done: task.plan.iter().filter(|step| step.done).count(),
                            steps: task.plan.len(),
                            gaps: None,
                            helpers: None,
                        },
                        (workspace, task.status.conversation.clone()),
                    ))
                })
                .unzip()
        };
        let titles: Vec<String> = jobs.iter().map(|job| job.title.clone()).collect();
        if !asks_for_status(&request.text, &titles) {
            return None;
        }
        // Read only now, outside the lock: the last check's gaps, and the
        // team's helpers.
        for (job, (workspace, conversation)) in jobs.iter_mut().zip(&found) {
            job.gaps = std::fs::read_to_string(workspace.join(goal::CHECK_FILE))
                .ok()
                .filter(|check| check.trim() != "verified");
            job.helpers = helpers_of(conversation);
        }
        let language = crate::assistant::lines::language(&request.text);
        let text = line(&jobs, &request.text, language);
        let conversation = crate::assistant::store::new_id();
        self.emit(&Event::Started {
            conversation: conversation.clone(),
            origin: Origin::ActionShortcut,
            question: request.text.clone(),
        });
        self.emit(&Event::Answer {
            conversation: conversation.clone(),
            origin: Origin::ActionShortcut,
            text: text.clone(),
        });
        self.voice.announce(
            self,
            super::voice::Priority::Answer,
            &conversation,
            "",
            &text,
        );
        Some(conversation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles() -> Vec<String> {
        vec![
            "Online-Shop wie Nike bauen".into(),
            "Einstein recherchieren".into(),
        ]
    }

    #[test]
    fn asking_how_the_jobs_do_is_told_from_other_questions() {
        for asked in [
            "Status?",
            "Wie weit ist der Shop?",
            "Was machen die Aufgaben?",
            "wie weit ist die Aufgabe",
            "Wie steht es um Einstein?",
            "What are the tasks doing?",
            "How far is the shop?",
            "progress",
        ] {
            assert!(asks_for_status(asked, &titles()), "{asked}");
        }
        for asked in [
            "Wie weit ist es nach Berlin?",
            "How far is the moon?",
            "Wie spät ist es?",
            "Recherchiere die besten Laptops",
            "Bau mir eine Seite, die den Fortschritt der Lieferungen in einer Tabelle zeigt",
        ] {
            assert!(!asks_for_status(asked, &titles()), "{asked}");
        }
    }

    fn job(title: &str, done: usize, steps: usize, gaps: Option<&str>) -> Job {
        Job {
            title: title.into(),
            state: TaskState::Running,
            elapsed_ms: 3 * 60 * 60_000,
            done,
            steps,
            gaps: gaps.map(str::to_owned),
            helpers: None,
        }
    }

    #[test]
    fn the_status_names_progress_time_and_what_is_open() {
        let jobs = [
            job(
                "Online-Shop wie Nike bauen",
                4,
                9,
                Some("- Seite läuft noch nicht online\n- kein GitHub"),
            ),
            job("Einstein recherchieren", 3, 3, None),
        ];
        assert_eq!(
            line(&jobs, "Wie weit ist der Shop?", Some("de")),
            "Online-Shop wie Nike bauen: läuft seit 3 Stunden, 4 von 9 Schritten erledigt. Noch offen: Seite läuft noch nicht online."
        );
        let all = line(&jobs, "status", Some("en"));
        assert!(
            all.contains("Einstein recherchieren: running for 3 hours, 3 of 3 steps done."),
            "{all}"
        );
        assert_eq!(
            line(&[], "status", Some("de")),
            "Gerade läuft kein Auftrag."
        );
        let mut waiting = job("Bericht", 0, 0, None);
        waiting.state = TaskState::NeedsInput;
        assert_eq!(
            line(&[waiting], "status", Some("de")),
            "Bericht: wartet auf deine Antwort."
        );
        let mut team = job("50 Lieferanten vergleichen", 1, 4, None);
        team.helpers = Some((3, 5));
        assert_eq!(
            line(&[team], "status", Some("de")),
            "50 Lieferanten vergleichen: läuft seit 3 Stunden, 1 von 4 Schritten erledigt, 3 von 5 Helfern fertig."
        );
        assert_eq!(duration(3 * 24 * 60 * 60_000, Some("de")), "3 Tagen");
        assert_eq!(duration(30_000, Some("en")), "a minute");
    }
}
