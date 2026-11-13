//! A job's team: helpers it starts for parts that can run beside each other,
//! so one request ("research 50 suppliers, put them in a sheet, message the
//! team the link") becomes hours of parallel work with one result.
//!
//! The job leading it (the *lead*) calls `hex_team_start` with each helper's
//! task; each helper is a Background job of its own (its own browsers,
//! apps, cursor, card at the orb, questions, and checked goal), working in a
//! folder inside the lead's, so the lead reads everything they make. The
//! lead follows them with `hex_team_status` (which can wait for the next to
//! finish), starts what depends on their results once they're in, and
//! answers once for all of it. Stopping the lead stops its helpers.
//!
//! Like the hand-off, this goes through small files under
//! `<assistant>/team/<lead>/`, since the tools server is its own process:
//!
//! ```text
//! requests/<nonce>.json   the tools server: jobs to start      (Ask)
//! answers/<nonce>.json    the runner: who started, or why not  (Answered)
//! members/<id>.json       the runner: each helper as it goes   (Member)
//! ```
//!
//! Membership and states live on disk, so a restart that resumes the lead
//! and its helpers keeps the team together.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::handoff::Job;
use crate::assistant::types::Event;

/// Helpers working at once for one lead on this computer
/// (`capacity::limits`: 32 on a 64 GB Mac).
pub fn max_at_once() -> usize {
    crate::assistant::capacity::limits().helpers
}
/// How long `hex_team_start` waits for the runner to take its request.
const ANSWER_WAIT: Duration = Duration::from_secs(20);
/// How long `hex_team_status` may wait for a helper to finish, per call.
pub const MAX_WAIT: Duration = Duration::from_secs(240);
/// A helper's answer kept for the lead, at most.
const ANSWER_KEPT: usize = 6_000;

/// Where a helper stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    /// It asked the person something and waits for the answer.
    Asking,
    Done,
    Failed,
    Stopped,
}

impl State {
    pub fn ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Stopped)
    }

    fn word(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Asking => "waiting for the person's answer",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// One helper, as the lead sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub title: String,
    pub task: String,
    pub state: State,
    #[serde(default)]
    pub step: Option<String>,
    /// Its final answer (or why it failed), once it ended.
    #[serde(default)]
    pub answer: Option<String>,
    /// When it was started (Unix ms), for the order.
    pub started_ms: u64,
}

/// A lead's request to start helpers, or to tell running ones something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ask {
    pub lead: String,
    pub nonce: String,
    pub jobs: Vec<Job>,
    /// New instructions for its helpers (`hex_team_tell`): each steers the
    /// helper it names, as the person's words would.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tell: Vec<Tell>,
}

/// One instruction for a helper.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tell {
    /// The helper's id, or `all`.
    pub to: String,
    pub text: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Answered {
    #[serde(default)]
    members: Vec<Member>,
    #[serde(default)]
    error: Option<String>,
}

/// `<assistant>/team`.
pub fn root() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dir = crate::assistant::store::assistant_dir().ok()?.join("team");
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

fn safe(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn dir(root: &Path, lead: &str, part: &str) -> Option<PathBuf> {
    if !safe(lead) {
        return None;
    }
    let dir = root.join(lead).join(part);
    crate::app_paths::create_private_dir(&dir).ok()?;
    Some(dir)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    // Written whole and renamed, so a reader never sees half a file.
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, path).map_err(|error| error.to_string())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn nonce() -> String {
    crate::assistant::store::new_id()[..8].to_owned()
}

// ---------------------------------------------------------------------------
// The tools server's side (the lead's agent)

/// Asks the runner to start `jobs` as helpers of `lead` and waits for them
/// to start; the helpers, or why they couldn't.
pub fn start(root: &Path, lead: &str, jobs: Vec<Job>) -> Result<Vec<Member>, String> {
    start_waiting(root, lead, jobs, ANSWER_WAIT)
}

/// Asks the runner to give `lead`'s helpers new instructions; the helpers
/// told, or why not.
pub fn tell(root: &Path, lead: &str, tell: Vec<Tell>) -> Result<Vec<Member>, String> {
    if tell.iter().all(|tell| tell.text.trim().is_empty()) {
        return Err("hex_team_tell needs “text”".into());
    }
    send(root, lead, Vec::new(), tell, ANSWER_WAIT)
}

fn start_waiting(
    root: &Path,
    lead: &str,
    jobs: Vec<Job>,
    wait: Duration,
) -> Result<Vec<Member>, String> {
    if jobs.is_empty() {
        return Err("hex_team_start needs at least one job with a “task”".into());
    }
    send(root, lead, jobs, Vec::new(), wait)
}

fn send(
    root: &Path,
    lead: &str,
    jobs: Vec<Job>,
    tell: Vec<Tell>,
    wait: Duration,
) -> Result<Vec<Member>, String> {
    let requests = dir(root, lead, "requests").ok_or("this task has no id Hex can use")?;
    let answers = dir(root, lead, "answers").ok_or("this task has no id Hex can use")?;
    let nonce = nonce();
    let ask = Ask {
        lead: lead.to_owned(),
        nonce: nonce.clone(),
        jobs,
        tell,
    };
    write_json(&requests.join(format!("{nonce}.json")), &ask)?;
    let answer = answers.join(format!("{nonce}.json"));
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if let Some(answered) = read_json::<Answered>(&answer) {
            let _ = std::fs::remove_file(&answer);
            return match answered.error {
                Some(error) => Err(error),
                None => Ok(answered.members),
            };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(requests.join(format!("{nonce}.json")));
    Err("Hex didn't take the helpers (is it running?). Do the work yourself instead.".into())
}

/// Every helper of `lead`, oldest first.
pub fn members(root: &Path, lead: &str) -> Vec<Member> {
    let Some(dir) = dir(root, lead, "members") else {
        return Vec::new();
    };
    let mut members: Vec<Member> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| read_json(&entry.path()))
        .collect();
    members.sort_by(|a, b| a.started_ms.cmp(&b.started_ms).then(a.id.cmp(&b.id)));
    members
}

/// `lead`'s helpers once one more of them ended than had when it was called
/// (or all have), or after `wait`, whichever comes first.
pub fn wait_for_change(root: &Path, lead: &str, wait: Duration) -> Vec<Member> {
    let ended = |members: &[Member]| members.iter().filter(|m| m.state.ended()).count();
    let asking = |members: &[Member]| members.iter().filter(|m| m.state == State::Asking).count();
    let first = members(root, lead);
    let (ended_before, asking_before) = (ended(&first), asking(&first));
    let until = Instant::now() + wait.min(MAX_WAIT);
    let mut now = first;
    while Instant::now() < until {
        if now.is_empty()
            || ended(&now) > ended_before
            || asking(&now) > asking_before
            || now.iter().all(|m| m.state.ended())
        {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
        now = members(root, lead);
    }
    now
}

/// What `hex_team_status` tells the lead.
pub fn report(members: &[Member]) -> String {
    if members.is_empty() {
        return "You have no helpers. Start some with hex_team_start.".into();
    }
    let count = |state: State| members.iter().filter(|m| m.state == state).count();
    let open = members.iter().filter(|m| !m.state.ended()).count();
    let mut out = format!(
        "{} helper(s): {} running, {} queued, {} waiting for the person, {} done, {} failed, {} stopped.\n",
        members.len(),
        count(State::Running),
        count(State::Queued),
        count(State::Asking),
        count(State::Done),
        count(State::Failed),
        count(State::Stopped),
    );
    for member in members {
        out.push_str(&format!(
            "\n## {} [{}] (id {})\n",
            member.title,
            member.state.word(),
            member.id
        ));
        if let Some(step) = member.step.as_deref().filter(|_| !member.state.ended()) {
            out.push_str(&format!("Now: {step}\n"));
        }
        if let Some(answer) = &member.answer {
            out.push_str(answer.trim());
            out.push('\n');
        }
    }
    out.push_str(if open == 0 {
        "\nEvery helper has ended: combine their results (their files are in their folders inside yours), check them, and finish."
    } else {
        "\nWhile they work, do your own part, or call hex_team_status with wait_seconds to wait for the next one to finish."
    });
    out
}

// ---------------------------------------------------------------------------
// The runner's side

/// Every lead's requests waiting to be taken, oldest first.
pub fn pending(root: &Path) -> Vec<Ask> {
    let mut asks: Vec<(std::time::SystemTime, Ask)> = Vec::new();
    for lead in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let requests = lead.path().join("requests");
        for entry in std::fs::read_dir(requests).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            match read_json::<Ask>(&path) {
                Some(ask) if safe(&ask.lead) && lead.file_name() == ask.lead.as_str() => {
                    asks.push((modified, ask));
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    asks.sort_by_key(|(modified, _)| *modified);
    asks.into_iter().map(|(_, ask)| ask).collect()
}

/// Answers `ask`: the helpers started (each recorded as a member), or why
/// none were.
pub fn answer(root: &Path, ask: &Ask, result: Result<Vec<Member>, String>) {
    if let Some(requests) = dir(root, &ask.lead, "requests") {
        let _ = std::fs::remove_file(requests.join(format!("{}.json", ask.nonce)));
    }
    let answered = match result {
        Ok(members) => {
            // A tell's helpers keep their records; only new ones are noted.
            for member in members.iter().filter(|_| ask.tell.is_empty()) {
                record(root, &ask.lead, member);
                index().insert(member.id.clone(), ask.lead.clone());
            }
            Answered {
                members,
                error: None,
            }
        }
        Err(error) => Answered {
            members: Vec::new(),
            error: Some(error),
        },
    };
    if let Some(answers) = dir(root, &ask.lead, "answers")
        && let Err(error) = write_json(&answers.join(format!("{}.json", ask.nonce)), &answered)
    {
        tracing::warn!(%error, "couldn't answer a team request");
    }
}

fn record(root: &Path, lead: &str, member: &Member) {
    if !safe(&member.id) {
        return;
    }
    if let Some(members) = dir(root, lead, "members")
        && let Err(error) = write_json(&members.join(format!("{}.json", member.id)), member)
    {
        tracing::warn!(%error, "couldn't record a helper");
    }
}

/// helper → lead, for every helper on disk (read once) and every one
/// started since.
fn index() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    static INDEX: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            let mut index = HashMap::new();
            if let Some(root) = root() {
                for lead in std::fs::read_dir(&root).into_iter().flatten().flatten() {
                    let Some(lead_id) = lead.file_name().to_str().map(str::to_owned) else {
                        continue;
                    };
                    for member in members(&root, &lead_id) {
                        index.insert(member.id, lead_id.clone());
                    }
                }
            }
            Mutex::new(index)
        })
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// The lead `conversation` helps, if it's a helper.
pub fn lead_of(conversation: &str) -> Option<String> {
    index().get(conversation).cloned()
}

/// Keeps a helper's record up to date with what its task reports.
pub fn on_event(root: &Path, event: &Event) {
    let Some(conversation) = event.conversation() else {
        return;
    };
    let Some(lead) = lead_of(conversation) else {
        return;
    };
    let Some(path) =
        dir(root, &lead, "members").map(|dir| dir.join(format!("{conversation}.json")))
    else {
        return;
    };
    let Some(mut member) = read_json::<Member>(&path) else {
        return;
    };
    let before = member.clone();
    apply(&mut member, event);
    if member != before {
        record(root, &lead, &member);
    }
}

/// What `event` changes about `member`.
fn apply(member: &mut Member, event: &Event) {
    match event {
        Event::Queued { .. } => member.state = State::Queued,
        Event::Started { .. } | Event::Resumed { .. } => member.state = State::Running,
        Event::Step { description, .. } => member.step = Some(description.clone()),
        Event::Question { .. } => member.state = State::Asking,
        Event::Answer { text, .. } => {
            member.state = State::Done;
            member.answer = Some(crate::assistant::store::shorten(text, ANSWER_KEPT));
        }
        Event::Failed { message, .. } => {
            member.state = State::Failed;
            member.answer = Some(message.clone());
        }
        Event::Cancelled { .. } => member.state = State::Stopped,
        _ => {}
    }
}

/// What a lead learns in its rules: when and how to use helpers, at most
/// `max` at once.
pub fn rules(max: usize) -> String {
    format!(
        "\nA team for big jobs: when the work splits into parts that can run beside each other (many sites or suppliers to research, several deliverables, a build beside a comparison), start helpers with hex_team_start: each is a full background job with its own browsers, apps, and folder inside yours, and asks the person on its own what it must know. When the person asks for a number of helpers or browsers (\"20 Agenten\", \"20 Browser\"), start that many helpers in ONE hex_team_start call right away, up to {max} at once on this computer, before doing anything else; they all start together. Give each a self-contained task (every detail it needs, and which files to write in its folder). A shared table (suppliers, leads, prices): give every helper the same columns and have each write its rows to its own part file in its folder (rows.csv with a header row), never into one shared sheet at the same time; once they're done, combine the parts yourself, without duplicates (same website or name), into one online sheet in your `signedIn: true` browser where the person's accounts have one (import the combined CSV, or paste it) and give the person its link; if they asked for an Excel or Numbers file, or have no such account, make that file with the apps and tools this Mac has. A step that needs their results (a message with the final link) comes after them: do it yourself once they're done, or start a helper for it then, naming the files to use. While they work, do your own part, or call hex_team_status with wait_seconds (up to 240) to wait for the next to finish; never poll it quickly. When the person changes what they want while helpers work, pass it on with hex_team_tell (one helper's id, or \"all\"). Read their answers and files, check them, and finish with one answer for all of it. Small jobs need no team.\n"
    )
}

/// What a helper learns in its rules: it does one part of its lead's job.
pub fn helper_rules(lead: &str) -> String {
    format!(
        "\nYou are a helper: the job “{lead}” gave you one part of it and combines what its helpers make. Do only your part, fully. Put your results as files in your folder (the lead reads it), and end with a short summary naming each file and any links. When your part is rows for a shared table, write them to the part file your lead named (rows.csv with a header row) with exactly its columns, adding rows as you find them; never write into the shared sheet yourself. For reading pages use your light browsers (several at once); real Chrome (`signedIn: true`) only for a site that needs a login or doesn't work in the light one. Don't start helpers of your own. Ask the person only what you can't find out and your lead couldn't have told you.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hex-team-{}-{}",
            std::process::id(),
            crate::assistant::store::new_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn job(title: &str) -> Job {
        Job {
            title: title.into(),
            task: format!("Research {title}"),
        }
    }

    fn member(id: &str, title: &str, started_ms: u64) -> Member {
        Member {
            id: id.into(),
            title: title.into(),
            task: format!("Research {title}"),
            state: State::Queued,
            step: None,
            answer: None,
            started_ms,
        }
    }

    #[test]
    fn a_lead_s_request_is_taken_and_answered_with_its_helpers() {
        let root = root();
        let runner_root = root.clone();
        // The runner, on its own thread, as the app's monitor would.
        let runner = std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(ask) = pending(&runner_root).into_iter().next() {
                    assert_eq!(ask.lead, "lead-1");
                    assert_eq!(ask.jobs.len(), 2);
                    let members = ask
                        .jobs
                        .iter()
                        .enumerate()
                        .map(|(n, job)| member(&format!("h{n}"), &job.title, n as u64))
                        .collect();
                    answer(&runner_root, &ask, Ok(members));
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!("no request came");
        });
        let started = start(&root, "lead-1", vec![job("Nike"), job("Adidas")]).unwrap();
        runner.join().unwrap();
        assert_eq!(started.len(), 2);
        assert!(pending(&root).is_empty(), "taken once");
        assert_eq!(
            members(&root, "lead-1")
                .iter()
                .map(|m| m.title.as_str())
                .collect::<Vec<_>>(),
            ["Nike", "Adidas"]
        );
        assert_eq!(lead_of("h1").as_deref(), Some("lead-1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refusal_reaches_the_lead_and_nobody_answering_times_out() {
        let root = root();
        let runner_root = root.clone();
        let runner = std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(ask) = pending(&runner_root).into_iter().next() {
                    answer(
                        &runner_root,
                        &ask,
                        Err("Only a job can lead a team.".into()),
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let refused = start(&root, "chat", vec![job("x")]).unwrap_err();
        runner.join().unwrap();
        assert_eq!(refused, "Only a job can lead a team.");
        let silent =
            start_waiting(&root, "lead-2", vec![job("x")], Duration::from_millis(300)).unwrap_err();
        assert!(silent.contains("Do the work yourself"), "{silent}");
        assert!(
            pending(&root).is_empty(),
            "an unanswered request is taken back"
        );
        assert!(start(&root, "../evil", vec![job("x")]).is_err());
        assert!(start(&root, "lead-3", vec![]).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn events_move_a_helper_along_and_the_report_tells_the_lead() {
        let mut m = member("h1", "Nike", 1);
        apply(
            &mut m,
            &Event::Started {
                conversation: "h1".into(),
                origin: crate::assistant::Origin::Background,
                question: "x".into(),
            },
        );
        assert_eq!(m.state, State::Running);
        apply(
            &mut m,
            &Event::Step {
                conversation: "h1".into(),
                description: "Opened nike.com".into(),
            },
        );
        assert_eq!(m.step.as_deref(), Some("Opened nike.com"));
        let running = report(std::slice::from_ref(&m));
        assert!(running.contains("Now: Opened nike.com"), "{running}");
        assert!(running.contains("wait_seconds"), "{running}");
        apply(
            &mut m,
            &Event::Answer {
                conversation: "h1".into(),
                origin: crate::assistant::Origin::Background,
                text: "Found 12 prices. Files: /x/nike".into(),
            },
        );
        assert_eq!(m.state, State::Done);
        let done = report(&[m]);
        assert!(
            done.contains("[done]") && done.contains("Found 12 prices"),
            "{done}"
        );
        assert!(done.contains("Every helper has ended"), "{done}");
        assert!(report(&[]).contains("no helpers"));
    }

    #[test]
    fn a_helper_knows_its_lead_and_its_part() {
        let rules = helper_rules("50 Lieferanten vergleichen");
        assert!(rules.contains("\u{201c}50 Lieferanten vergleichen\u{201d}"));
        assert!(rules.contains("Don't start helpers"));
        assert!(rules.contains("rows.csv"), "a shared table goes in parts");
    }

    #[test]
    fn a_lead_starts_as_many_helpers_as_asked_at_once_and_merges_their_parts() {
        let rules = rules(32);
        assert!(rules.contains("up to 32 at once"));
        assert!(rules.contains("ONE hex_team_start call"));
        assert!(rules.contains("online sheet"));
        assert!(!rules.contains("at most 12"));
        assert!((2..=crate::assistant::capacity::MAX_HELPERS).contains(&max_at_once()));
    }

    #[test]
    fn waiting_returns_as_soon_as_one_more_helper_ends() {
        let root = root();
        let mut a = member("wa", "A", 1);
        a.state = State::Running;
        let b = Member {
            state: State::Running,
            ..member("wb", "B", 2)
        };
        record(&root, "lead-w", &a);
        record(&root, "lead-w", &b);
        let writer_root = root.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            a.state = State::Done;
            record(&writer_root, "lead-w", &a);
        });
        let started = Instant::now();
        let now = wait_for_change(&root, "lead-w", Duration::from_secs(20));
        writer.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(now.iter().filter(|m| m.state.ended()).count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
