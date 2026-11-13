//! The service's events as the Assistant pane takes them in: the shared
//! task view (`assistant::view`, which the overlays read too) decides what
//! each task does; the pane keeps the turns the person made here, follows
//! new activity, and reloads the stored list when it changes.

use std::time::Instant;

use super::{ChatsPane, Turn};
use crate::assistant::view::{Phase, Task};
use crate::assistant::{Event, PlanStep};

impl ChatsPane {
    /// Takes in one event. True when what the pane shows changed.
    pub(in crate::rune_ui) fn apply(&mut self, event: &Event) -> bool {
        let now = Instant::now();
        let mut reload_list = false;
        let mut reload_shown = false;
        if let Some(id) = event.conversation()
            && self.view.task(id).is_none()
        {
            // A task that started before the pane listened.
            self.view.adopt(&self.tasks);
        }
        self.view.apply(event, None, now);
        match event {
            Event::Queued {
                conversation,
                origin,
                question,
            }
            | Event::Started {
                conversation,
                origin,
                question,
            } => {
                let new = !self.turns.contains_key(conversation);
                let turn = self
                    .turns
                    .entry(conversation.clone())
                    .or_insert_with(|| Turn::new(question.clone(), *origin));
                if !new && matches!(event, Event::Started { .. }) {
                    // Answered (by voice, the orb, or here): the question
                    // and its answer are stored messages now, so they don't
                    // show again in flight.
                    turn.answers.clear();
                }
                reload_list = !self.list.iter().any(|row| &row.id == conversation);
                if new {
                    // A task started, or a question was asked.
                    self.activity(conversation, now);
                }
            }
            Event::Answer { conversation, .. } | Event::Cancelled { conversation } => {
                self.turns.remove(conversation);
                // Stored now; the pane keeps no finished task.
                self.view.forget(conversation);
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::Failed { conversation, .. } => {
                // Its failure stays in the view until it's asked again.
                self.turns.remove(conversation);
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::ConversationsChanged => {
                reload_list = true;
                reload_shown = true;
            }
            Event::Question { conversation, .. } => {
                // It waits on the person: show it, its answer field ready.
                self.activity(conversation, now);
                if self.selected.as_ref() == Some(conversation) {
                    self.focus_answer = true;
                    self.follow.to_end = true;
                }
            }
            Event::Escalated { conversation } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.origin = crate::assistant::Origin::Background;
                }
                reload_list = true;
            }
            // The view has them; the overlays draw the screen's part.
            Event::Thinking { .. }
            | Event::Step { .. }
            | Event::Window { .. }
            | Event::AnswerDelta { .. }
            | Event::Paused { .. }
            | Event::Resumed { .. }
            | Event::Waiting { .. }
            | Event::HandsOff { .. }
            | Event::Acting { .. }
            | Event::Plan { .. }
            | Event::QuestionsChanged { .. }
            | Event::Understood { .. } => {}
        }
        if matches!(
            event,
            Event::Step { .. }
                | Event::Window { .. }
                | Event::AnswerDelta { .. }
                | Event::Answer { .. }
                | Event::Failed { .. }
        ) && let Some(conversation) = event.conversation()
        {
            self.more_in(conversation);
        }
        if reload_list {
            self.list = self.source.conversations();
            self.listed = true;
            self.refresh_stream();
        }
        if reload_shown {
            self.reload_shown();
            // A deleted conversation leaves the transcript for a new chat.
            if self.shown.is_none()
                && let Some(id) = &self.selected
                && !self.turns.contains_key(id)
                && self.failure(id).is_none()
                && !self.list.iter().any(|row| &row.id == id)
            {
                self.selected = None;
            }
        }
        self.tasks = self.source.tasks();
        self.view.adopt(&self.tasks);
        true
    }

    /// Task `id` while it's live, as its events left it.
    pub(super) fn in_flight(&self, id: &str) -> Option<&Task> {
        self.view.live_task(id)
    }

    /// Why conversation `id`'s last request failed, until it's asked again.
    pub(super) fn failure(&self, id: &str) -> Option<&str> {
        self.view
            .task(id)
            .filter(|task| task.phase == Phase::Failed)
            .and_then(|task| task.error.as_deref())
    }

    /// Every conversation whose last request failed.
    pub(super) fn failed(&self) -> impl Iterator<Item = &str> {
        self.view
            .tasks
            .iter()
            .filter(|task| task.phase == Phase::Failed && task.error.is_some())
            .map(|task| task.id.as_str())
    }

    /// Running task `id`'s plan, once it made one.
    pub(super) fn plan(&self, id: &str) -> Option<&[PlanStep]> {
        self.in_flight(id)
            .map(|task| task.plan.as_slice())
            .filter(|plan| !plan.is_empty())
    }

    /// What running task `id` asked the person and waits on.
    pub(super) fn question(&self, id: &str) -> Option<&str> {
        self.in_flight(id)?.question.as_deref()
    }

    /// Whether task `id` moves the real pointer or keys now.
    pub(super) fn hands_off(&self, id: &str) -> bool {
        self.in_flight(id).is_some_and(|task| task.hands_off)
    }

    /// Whether `id` was a quick turn that moved to the background.
    pub(super) fn escalated(&self, id: &str) -> bool {
        self.view.task(id).is_some_and(|task| task.escalated)
    }
}
