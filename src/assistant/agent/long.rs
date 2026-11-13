//! What keeps a job going for hours (`agent::run_with`): its journal, the
//! measure of its backend session and the fresh sessions it starts
//! (`rollover`), and its retries after the network went away or the
//! backend ended (`recovery`). One per supervised run, alive across its
//! rounds and sessions.
//!
//! OWNER: brain.

use std::path::{Path, PathBuf};

use super::journal::{Journal, Kind};
use super::rollover::{self, Limits, Meter, Usage, Why};
use super::{Job, JobKind};
use crate::assistant::provider::Control;

/// A supervised job's long-run state.
#[derive(Debug)]
pub struct Long {
    pub workspace: PathBuf,
    pub journal: Journal,
    pub limits: Limits,
    pub meter: Meter,
    /// Fresh sessions this run started.
    pub rollovers: u32,
    /// Why the session was ended for a fresh one, until it is.
    pub due: Option<Why>,
    /// Failures in a row without a step between them.
    pub retries: u32,
}

impl Long {
    /// `job`'s, in `workspace`, at `now_ms`: its journal opened (started
    /// when it is new), and what brought this run noted.
    pub fn open(job: &Job, workspace: &Path, limits: Limits, now_ms: u64) -> Self {
        let mut journal = Journal::open(
            workspace,
            now_ms,
            &job.request.text,
            job.settings.job_budget_minutes,
        );
        match &job.kind {
            JobKind::Resume => journal.note(now_ms, Kind::Resumed, "after Agent-Desk restarted"),
            JobKind::AfterLimit => journal.note(now_ms, Kind::Back, "the usage limit lifted"),
            JobKind::Steer { .. } => journal.note(
                now_ms,
                Kind::Resumed,
                &format!(
                    "with the person's new instruction: “{}”",
                    crate::assistant::store::shorten(job.request.text.trim(), 160)
                ),
            ),
            _ => {}
        }
        Self {
            workspace: workspace.to_path_buf(),
            journal,
            limits,
            meter: Meter::new(limits),
            rollovers: 0,
            due: None,
            retries: 0,
        }
    }

    /// Whether the saved session is already past its size: a resumed job
    /// then starts fresh instead of resuming it.
    pub fn saved_too_large(&self) -> Option<String> {
        let tokens = self.journal.state.context_tokens;
        (tokens >= self.limits.context_tokens)
            .then(|| format!("the saved session was already at {}k tokens", tokens / 1000))
    }

    /// A step ended with the session at `usage`: noted, its milestones
    /// recorded, and the session ended for a fresh one when it is due.
    pub fn step(&mut self, usage: Usage, now_ms: u64, control: &Control) {
        self.retries = 0;
        self.journal.sized(usage.context_tokens);
        self.journal.milestones(now_ms);
        let due = self.meter.step(usage, now_ms);
        if let Some(why) = due
            && self.due.is_none()
            && self.rollovers < rollover::MAX_ROLLOVERS
        {
            tracing::info!(why = why.words(), "a long job goes on in a fresh session");
            self.due = Some(why);
            control.rotate();
        }
    }

    /// A tool call: a browser's page is kept for a fresh session.
    pub fn tool(&mut self, name: &str, input: &str) {
        if !name.contains("browser") {
            return;
        }
        let Ok(input) = serde_json::from_str::<serde_json::Value>(input) else {
            return;
        };
        if let Some(url) = input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .filter(|url| url.starts_with("http"))
        {
            let session = input
                .get("session")
                .and_then(serde_json::Value::as_str)
                .filter(|session| !session.is_empty())
                .unwrap_or("main");
            self.journal.page(session, url);
        }
    }

    /// Starts the next session after `why`: noted, measured afresh; its
    /// first message.
    pub fn roll_over(&mut self, job: &Job, why: &str, now_ms: u64) -> String {
        self.rollovers += 1;
        self.retries = 0;
        self.due = None;
        self.journal.milestones(now_ms);
        self.journal.new_session(now_ms, why);
        self.meter = Meter::new(self.limits);
        let seed = rollover::seed(job, &self.workspace, &self.journal, why, now_ms);
        self.journal.take_pending();
        seed
    }

    /// What a run that goes on in its session is told after `note`: the
    /// note, then the time, plan, journal, and what is open.
    pub fn go_on(&mut self, note: &str, now_ms: u64) -> String {
        let context = rollover::context(&self.workspace, &self.journal, now_ms);
        self.journal.take_pending();
        format!("{}\n\n{context}", note.trim_end())
    }

    /// What a check's next instruction gets: the time (and that it is
    /// nearly out), and what Agent-Desk found since.
    pub fn reminder(&mut self, now_ms: u64) -> String {
        self.journal.milestones(now_ms);
        let mut text = self.journal.time_line(now_ms);
        if self.journal.nearly_out(now_ms) {
            text.push('\n');
            text.push_str(rollover::NEARLY_OUT);
        }
        for pending in self.journal.take_pending() {
            text.push_str(&format!(
                "\nAgent-Desk found: {pending}. Do that step again another way."
            ));
        }
        text
    }
}
