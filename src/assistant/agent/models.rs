//! Which model and effort a task runs on, and when a quick turn on the
//! fast model hands over to the strong one.

use std::time::Duration;

use crate::app_settings::{AssistantBackend, AssistantSettings};

use super::Job;
use crate::assistant::provider::{self, Run, limit};
use crate::assistant::types::Lane;

/// The model `job` runs on: the Here lane's fast one where the backend
/// takes it, else the backend's own.
pub fn model(job: &Job) -> Option<String> {
    let settings = &job.settings;
    let here = job.request.origin.lane() == Lane::Here;
    let fast = settings
        .here_model
        .as_deref()
        .map(str::trim)
        .filter(|model| here && !model.is_empty())
        .map(str::to_owned);
    match settings.backend {
        AssistantBackend::ClaudeCode => fast.or_else(|| settings.claude_code_model.clone()),
        AssistantBackend::Codex => settings.codex_model.clone(),
        AssistantBackend::OpenCode => crate::assistant::backends::opencode_model(settings),
        AssistantBackend::ClaudeApi => fast.or_else(|| Some(settings.claude_api_model.clone())),
    }
}

/// Whether a fast model's failed turn goes on with the strong one: it
/// handed over ([`provider::Escalate`]) or failed, but not on a usage
/// limit (the strong model shares it) nor when it was stopped.
pub(super) fn should_escalate(error: &color_eyre::Report) -> bool {
    provider::escalated(error).is_some()
        || (limit::detect(&format!("{error:#}")).is_none() && format!("{error:#}") != "cancelled")
}

/// The model a Here turn on the fast one goes on with when it hands over
/// ([`provider::Escalate`]): Claude Code's own (`None`: its default).
pub fn strong_model(settings: &AssistantSettings) -> Option<String> {
    settings
        .claude_code_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
}

/// Whether `job` runs on the Here lane's fast model on Claude Code, where
/// a hard turn can hand over to the strong one.
pub(super) fn on_fast_model(job: &Job) -> bool {
    job.settings.backend == AssistantBackend::ClaudeCode
        && job.request.origin.lane() == Lane::Here
        && model(job).is_some_and(|fast| Some(&fast) != strong_model(&job.settings).as_ref())
}

/// After how many tool calls a Here turn on the fast model hands over to
/// the strong one (`assistant.escalate_after_tools`); `None` never.
pub fn escalate_after(job: &Job) -> Option<usize> {
    let after = job.settings.escalate_after_tools as usize;
    (after > 0 && on_fast_model(job)).then_some(after)
}

/// How hard a run of `job` thinks: the Here lane's effort on Claude Code
/// (`assistant.here_effort`), else the model's own.
pub fn effort(job: &Job) -> Option<String> {
    (job.settings.backend == AssistantBackend::ClaudeCode
        && job.request.origin.lane() == Lane::Here)
        .then(|| job.settings.here_effort.clone())
        .flatten()
        .filter(|effort| !effort.trim().is_empty())
}

/// What the strong model is told when it takes a turn over from the fast
/// one, in the same session.
pub const ESCALATED_NOTE: &str = "You are taking this turn over from a faster model, in the same session: everything above happened. Check where it stands (take a fresh look where it matters), then finish the person's request; never repeat a send or submit that may already have happened. Your answer is read aloud: keep it short and speakable.";

/// The run that hands `run` (a fast model's turn that stopped or failed)
/// to the strong model: the same session, a note to go on.
pub(super) fn escalated(run: &Run, job: &Job, session: Option<String>) -> Run {
    let mut strong = run.clone();
    strong.model = strong_model(&job.settings);
    strong.effort = None;
    strong.escalate_after = None;
    // Its process has other arguments than the next fast turn's: keeping it
    // would only wait to expire.
    strong.keep_warm = None;
    strong.image = None;
    strong.resume = session;
    strong.prompt = ESCALATED_NOTE.to_owned();
    strong
}

/// How long a Here process waits after its answer: for the follow-up
/// window (the next new conversation's is started by `service::stand_by`).
/// `None`: it ends.
pub(super) fn keep_warm_for(settings: &AssistantSettings) -> Option<Duration> {
    let seconds = u64::from(settings.follow_up_seconds);
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

/// How long a backend started ahead waits for its request.
pub(super) const AHEAD: Duration = Duration::from_secs(60);

/// How long a backend started ahead for the next `lane` request (for a
/// `new` conversation or not) waits for it: a minute, and a new Here
/// conversation's `assistant.warm_minutes` (the always-warm cap).
pub fn ahead_for(settings: &AssistantSettings, lane: Lane, new: bool) -> Duration {
    let warm = Duration::from_secs(u64::from(settings.warm_minutes) * 60);
    if lane == Lane::Here && new {
        warm.max(AHEAD)
    } else {
        AHEAD
    }
}
