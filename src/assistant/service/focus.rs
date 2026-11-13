//! Which conversation a hold or a follow-up goes to, and which running
//! job a message names.

use super::*;

/// `focus` on `state`: the inbox's active question, a running Here task,
/// Chats' conversation, or the last Here answer within `follow_up_seconds`.
pub(super) fn focus_in(state: &State, follow_up_seconds: u32, now: u64) -> Option<ConversationId> {
    if let Some(active) = inbox::of(state).active {
        return Some(active.conversation);
    }
    focus_past(state, follow_up_seconds, now)
}

/// `focus_in` past the inbox: a running Here task, Chats' conversation, or
/// the last Here answer within `follow_up_seconds`.
pub(super) fn focus_past(state: &State, follow_up_seconds: u32, now: u64) -> Option<ConversationId> {
    if let Some(task) = state
        .tasks
        .iter()
        .find(|task| task.lane() == Lane::Here && !task.control.cancelled())
    {
        return Some(task.status.conversation.clone());
    }
    // One assistant, one stream: what Chats shows never decides where a
    // hold goes. The assistant reads the stream (`context::digest`) and
    // steers a job with `hex_tell_job` when the person means one.
    let (id, answered) = state.last_quick.as_ref()?;
    (follow_up_seconds > 0 && now.saturating_sub(*answered) <= u64::from(follow_up_seconds) * 1000)
        .then(|| id.clone())
}

/// The one task among `running` that `to` names: its short id (as the
/// digest shows it, with or without brackets), else the only title that
/// contains it.
pub(super) fn job_named(to: &str, running: &[TaskStatus]) -> Option<ConversationId> {
    let to = to.trim().trim_matches(['[', ']']).trim();
    if to.is_empty() {
        return None;
    }
    if let Some(task) = running
        .iter()
        .find(|task| task.conversation.starts_with(to))
    {
        return Some(task.conversation.clone());
    }
    let lower = to.to_lowercase();
    let mut titled = running
        .iter()
        .filter(|task| task.title.to_lowercase().contains(&lower));
    match (titled.next(), titled.next()) {
        (Some(task), None) => Some(task.conversation.clone()),
        _ => None,
    }
}
