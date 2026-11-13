//! The service's events as the Chats pane takes them in: a request's turn
//! from Queued to its answer, a task taken over or waiting on a usage
//! limit, and the stored list changing.

use std::time::Instant;

use super::{ChatsPane, Hold, MAX_STEPS, Phase, Turn};
use crate::assistant::Event;
use crate::desktop::i18n::tr_fill;

impl ChatsPane {
    /// Takes in one event. True when what the pane shows changed.
    pub(in crate::rune_ui) fn apply(&mut self, event: &Event) -> bool {
        let mut reload_list = false;
        let mut reload_shown = false;
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
                let phase = if matches!(event, Event::Queued { .. }) {
                    Phase::Queued
                } else {
                    Phase::Thinking
                };
                let new = !self.turns.contains_key(conversation);
                let turn = self
                    .turns
                    .entry(conversation.clone())
                    .or_insert_with(|| Turn::new(question.clone(), *origin, phase));
                turn.phase = phase;
                if !new && matches!(event, Event::Started { .. }) {
                    // It goes on: what it said so far is stored now, so
                    // the text in flight starts afresh (else it doubles).
                    turn.answer.clear();
                    turn.steps.clear();
                    // Answered (by voice, the orb, or here): the question's
                    // box goes; the question and its answer are stored
                    // messages now, so they don't show again in flight.
                    self.questions.remove(conversation);
                    turn.answers.clear();
                }
                self.failures.remove(conversation);
                reload_list = !self.list.iter().any(|row| &row.id == conversation);
                if new {
                    // A task started, or a question was asked.
                    self.activity(conversation, Instant::now());
                }
            }
            Event::Thinking { conversation } => {
                self.retrying(conversation);
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Thinking;
                }
            }
            Event::Step {
                conversation,
                description,
            } => {
                self.retrying(conversation);
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Working;
                    if turn.steps.len() < MAX_STEPS {
                        turn.steps.push(description.clone());
                    }
                }
            }
            Event::Window {
                conversation,
                window,
            } => {
                if let Some(turn) = self.turns.get_mut(conversation)
                    && turn.steps.len() < MAX_STEPS
                {
                    turn.steps.push(tr_fill("Working in {}", &window.app));
                }
            }
            Event::AnswerDelta { conversation, text } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.phase = Phase::Answering;
                    turn.answer.push_str(text);
                }
            }
            Event::Answer { conversation, .. } | Event::Cancelled { conversation } => {
                self.ended(conversation);
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::Failed {
                conversation,
                message,
            } => {
                self.ended(conversation);
                self.failures.insert(conversation.clone(), message.clone());
                reload_list = true;
                reload_shown = self.selected.as_ref() == Some(conversation);
            }
            Event::Paused { conversation } => {
                self.holds.insert(conversation.clone(), Hold::Paused);
            }
            Event::Resumed { conversation } => {
                self.holds.remove(conversation);
            }
            Event::Waiting {
                conversation,
                until_ms,
                reason,
            } => {
                self.holds.insert(
                    conversation.clone(),
                    Hold::Waiting {
                        until_ms: *until_ms,
                        reason: reason.clone(),
                    },
                );
            }
            Event::HandsOff { conversation, on } => {
                if *on {
                    self.hands_off = Some(conversation.clone());
                } else if self.hands_off.as_ref() == Some(conversation) {
                    self.hands_off = None;
                }
            }
            // The overlay's frame follows it; Chats has nothing to show.
            Event::Acting { .. } => {}
            Event::ConversationsChanged => {
                reload_list = true;
                reload_shown = true;
            }
            Event::Plan {
                conversation,
                steps,
            } => {
                self.plans.insert(conversation.clone(), steps.clone());
            }
            Event::Question { conversation, text } => {
                self.questions.insert(conversation.clone(), text.clone());
                // What it said before asking is stored with the question.
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.answer.clear();
                }
                // It waits on the person: show it, its answer field ready.
                self.activity(conversation, Instant::now());
                if self.selected.as_ref() == Some(conversation) {
                    self.focus_answer = true;
                    self.follow.to_end = true;
                }
            }
            Event::Escalated { conversation } => {
                if let Some(turn) = self.turns.get_mut(conversation) {
                    turn.origin = crate::assistant::Origin::Background;
                }
                if !self.escalated.contains(conversation) {
                    self.escalated.push(conversation.clone());
                }
                reload_list = true;
            }
            // The screen frame shows it; the strip follows the tasks.
            Event::ModeChanged { .. } | Event::QuestionsChanged { .. } => {}
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
                && !self.failures.contains_key(id)
                && !self.list.iter().any(|row| &row.id == id)
            {
                self.selected = None;
            }
        }
        self.tasks = self.source.tasks();
        true
    }

    /// A task waiting on a usage limit moves again.
    fn retrying(&mut self, conversation: &str) {
        if matches!(self.holds.get(conversation), Some(Hold::Waiting { .. })) {
            self.holds.remove(conversation);
        }
    }

    /// A request ended: its turn, plan, question, hold, and Hands off go.
    fn ended(&mut self, conversation: &str) {
        self.turns.remove(conversation);
        self.holds.remove(conversation);
        self.plans.remove(conversation);
        self.questions.remove(conversation);
        if self.hands_off.as_deref() == Some(conversation) {
            self.hands_off = None;
        }
    }
}
