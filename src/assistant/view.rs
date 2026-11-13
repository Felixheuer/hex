//! What the assistant's surfaces show, as plain state fed by
//! [`Event`]s: one reducer that the overlays (`rune_ui::assistant_overlay`'s
//! board, orb, job rows, outlines, and frame) and the Assistant pane
//! (`rune_ui::chats`) both read, so a task's states (queued, running, taken
//! over, waiting out a usage limit, asking the person, a helper of a job,
//! answered, failed, stopped) are decided once, here.
//!
//! ```ts
//! service::subscribe ── Event ──> Stream::apply ──> Task (phase, activity, steps, log,
//!                                                       windows, answer, error, hold,
//!                                                       question, plan, lead, since)
//!                                     ├── assistant_overlay::board (pops, results, the inbox's bubble)
//!                                     └── rune_ui::chats (its turns: what the person sent and answered)
//! ```
//!
//! No drawing, no platform, no clock of its own (every call takes `now`):
//! every rule is a unit test.

mod task;
#[cfg(test)]
mod tests;
mod text;

use std::time::Instant;

pub use task::{
    ACTING_FOR, Activity, BACK_FOR, Borrow, FINISHED_LINGER, Hold, LOG_MAX, Logged, Phase, Place,
    STOPPED_LINGER, TASK_STEPS, Task,
};
pub use text::{LINE_CHARS, first_line, plain, shorten};

use crate::assistant::{ConversationId, Event, Lane, Origin, TaskStatus, WaitingQuestion};

/// How many task colours there are (`assistant::palette`); a colour index
/// wraps past it.
pub const COLOURS: u8 = crate::assistant::palette::COLOURS.len() as u8;

/// The service's inbox of questions as last told
/// (`Event::QuestionsChanged`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inbox {
    pub active: Option<WaitingQuestion>,
    /// Questions behind the active one.
    pub waiting: usize,
}

/// Every task a surface knows of, oldest first, and the inbox.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    pub tasks: Vec<Task>,
    /// Questions asked so far, to order them newest first.
    asked: u64,
    /// The inbox, once the service told it; before that the newest question
    /// stands for the active one.
    pub inbox: Option<Inbox>,
}

impl Stream {
    /// The tasks running or queued when the surface started listening.
    pub fn seeded(statuses: &[TaskStatus]) -> Self {
        let mut stream = Self::default();
        stream.adopt(statuses);
        stream
    }

    /// Takes in the listed tasks it doesn't know yet (they started before
    /// it listened); known ones keep what their events said. Whether any
    /// was new.
    pub fn adopt(&mut self, statuses: &[TaskStatus]) -> bool {
        let before = self.tasks.len();
        for status in statuses {
            if self.task(&status.conversation).is_some() {
                continue;
            }
            let mut task = Task::new(
                status.conversation.clone(),
                status.origin,
                &status.title,
                status.colour % COLOURS,
                Phase::of(status.state),
            );
            // Picked up part way: it's been at work.
            task.activity = Activity::Working;
            if let Some(step) = &status.step {
                task.note_step(step);
            }
            task.windows = status.windows.clone();
            self.tasks.push(task);
        }
        self.tasks.len() != before
    }

    /// Applies `event`; `colour` is the service's colour for a task it
    /// starts (`TaskStatus.colour`), when it has one. Whether what a glance
    /// shows changed (the answer streaming in doesn't count: only its end
    /// is read at a glance).
    pub fn apply(&mut self, event: &Event, colour: Option<u8>, now: Instant) -> bool {
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
                let started = matches!(event, Event::Started { .. });
                let phase = if started {
                    Phase::Running
                } else {
                    Phase::Queued
                };
                if let Some(task) = self.task_mut(conversation) {
                    // A follow-up in the same conversation starts it again.
                    let restarted = task.ended.is_some();
                    task.phase = phase;
                    task.activity = Activity::Thinking;
                    task.ended = None;
                    task.error = None;
                    if started || restarted {
                        // What it said so far is stored now: the text in
                        // flight starts afresh.
                        task.answer.clear();
                        task.log.clear();
                        task.understood = None;
                    }
                    if task.since.is_none() || restarted {
                        task.since = Some(now);
                    }
                    // Answered: a follow-up is how an answer arrives.
                    task.question = None;
                    if let Some(colour) = colour {
                        task.colour = colour % COLOURS;
                    }
                    return true;
                }
                let colour = colour.map_or_else(|| self.free_colour(), |c| c % COLOURS);
                let mut task = Task::new(conversation.clone(), *origin, question, colour, phase);
                task.since = Some(now);
                task.lead = crate::assistant::agent::team::lead_of(conversation);
                self.tasks.push(task);
                true
            }
            Event::Thinking { conversation } => self.live(conversation, |task| {
                let was = (task.phase, task.activity);
                task.phase = task.phase.resumed();
                task.activity = Activity::Thinking;
                (task.phase, task.activity) != was
            }),
            Event::Step {
                conversation,
                description,
            } => self.live(conversation, |task| {
                task.phase = task.phase.resumed();
                task.activity = Activity::Working;
                task.note_step(description);
                task.log(Logged::Step(description.clone()));
                true
            }),
            Event::Window {
                conversation,
                window,
            } => self.live(conversation, |task| {
                if task.windows.contains(window) {
                    return false;
                }
                task.windows.push(window.clone());
                task.log(Logged::Window(window.app.clone()));
                true
            }),
            Event::AnswerDelta { conversation, text } => self.live(conversation, |task| {
                task.activity = Activity::Answering;
                task.answer.push_str(text);
                false
            }),
            Event::Answer {
                conversation, text, ..
            } => self.finish(conversation, Phase::Done, now, |task| {
                task.answer = text.clone();
            }),
            Event::Failed {
                conversation,
                message,
            } => {
                if self.task(conversation).is_none() {
                    // Failed before it was seen to start: keep why, for the
                    // surfaces that say it.
                    let mut task =
                        Task::new(conversation.clone(), Origin::Chat, "", 0, Phase::Running);
                    task.lead = crate::assistant::agent::team::lead_of(conversation);
                    self.tasks.push(task);
                }
                self.finish(conversation, Phase::Failed, now, |task| {
                    task.error = Some(message.clone());
                })
            }
            Event::Cancelled { conversation } => {
                self.finish(conversation, Phase::Stopped, now, |_| {})
            }
            Event::Paused { conversation } => {
                self.live(conversation, |task| set(&mut task.phase, Phase::Paused))
            }
            Event::Resumed { conversation } => {
                self.live(conversation, |task| set(&mut task.phase, Phase::Running))
            }
            Event::Waiting {
                conversation,
                until_ms,
                reason,
            } => self.live(conversation, |task| {
                let phase = set(
                    &mut task.phase,
                    Phase::Waiting {
                        until_ms: *until_ms,
                    },
                );
                set(&mut task.reason, reason.clone()) || phase
            }),
            Event::HandsOff { conversation, on } => {
                self.live(conversation, |task| set(&mut task.hands_off, *on))
            }
            // Working on the screen: a desktop action by any task, or a
            // question about the screen reading a window.
            Event::Acting {
                conversation,
                looking,
            } => self.live(conversation, |task| {
                if *looking && task.origin.lane() != Lane::Here {
                    return false;
                }
                let was_on = task.acting_at.is_some_and(|at| now < at + ACTING_FOR);
                task.acting_at = Some(now);
                !was_on
            }),
            Event::Plan {
                conversation,
                steps,
            } => self.live(conversation, |task| set(&mut task.plan, steps.clone())),
            Event::Board {
                conversation,
                claimed,
                done,
            } => self.live(conversation, |task| {
                set(&mut task.board, Some((*claimed, *done)))
            }),
            Event::Question { conversation, text } => {
                self.asked += 1;
                let asked = self.asked;
                self.live(conversation, |task| {
                    task.phase = Phase::NeedsInput;
                    task.question = Some(text.trim().to_owned());
                    task.asked = asked;
                    // What it said before asking is stored with the question.
                    task.answer.clear();
                    true
                })
            }
            Event::Escalated { conversation } => self.live(conversation, |task| {
                // The quick turn continues as a Background task.
                let escalated = set(&mut task.escalated, true);
                set(&mut task.origin, Origin::Background) || escalated
            }),
            Event::QuestionsChanged { active, waiting } => set(
                &mut self.inbox,
                Some(Inbox {
                    active: active.clone(),
                    waiting: *waiting,
                }),
            ),
            Event::ConversationsChanged => false,
            Event::Understood { conversation, text } => self.live(conversation, |task| {
                set(&mut task.understood, Some(text.trim().to_owned()))
            }),
            Event::Borrowed {
                conversation,
                window,
                why,
                until_ms,
            } => self.live(conversation, |task| {
                task.returned = None;
                set(
                    &mut task.borrowed,
                    Some(task::Borrow {
                        window: window.clone(),
                        why: why.clone(),
                        until_ms: *until_ms,
                    }),
                )
            }),
            Event::Returned { conversation, to } => self.live(conversation, |task| {
                let was = task.borrowed.take().is_some();
                // Taken over (`to` none): the window is the person's, and
                // its dashed outline says so instead.
                task.returned = to.as_ref().map(|app| (app.clone(), now));
                was || task.returned.is_some()
            }),
            Event::InProject { conversation, name } => match self.task_mut(conversation) {
                Some(task) => {
                    task.suggested = None;
                    set(&mut task.project, Some(name.clone()))
                }
                None => false,
            },
            Event::ProjectSuggested {
                conversation,
                id,
                name,
            } => match self.task_mut(conversation) {
                Some(task) => set(&mut task.suggested, Some((id.clone(), name.clone()))),
                None => false,
            },
        }
    }

    /// Drops finished tasks whose time is up; whether any went.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.tasks.len();
        self.tasks
            .retain(|task| task.leaves_at().is_none_or(|at| now < at));
        self.tasks.len() != before
    }

    /// When the next finished task leaves.
    pub fn next_expiry(&self) -> Option<Instant> {
        self.tasks.iter().filter_map(Task::leaves_at).min()
    }

    /// Forgets `id` at once (a surface that keeps no finished task);
    /// whether it knew it.
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|task| task.id != id);
        self.tasks.len() != before
    }

    /// Forgets `id`'s failure (it's asked again, or deleted); a live task
    /// stays.
    pub fn clear_failure(&mut self, id: &str) {
        self.tasks
            .retain(|task| task.id != id || task.phase != Phase::Failed);
    }

    /// The person answered `id`'s question (it goes on with a follow-up):
    /// the question, taken.
    pub fn answered(&mut self, id: &str) -> Option<String> {
        self.task_mut(id)?.question.take()
    }

    /// The Background tasks the orb and its list show, oldest first.
    pub fn stack(&self) -> Vec<Task> {
        self.tasks
            .iter()
            // Jobs: a pane request that became one was escalated, which
            // made it Background here; a plain pane reply isn't one.
            .filter(|task| task.origin == Origin::Background)
            .cloned()
            .collect()
    }

    /// Whether any task waits out a usage limit (its countdown ticks).
    pub fn waiting(&self) -> bool {
        self.tasks
            .iter()
            .any(|task| matches!(task.phase, Phase::Waiting { .. }))
    }

    /// The live task moving the real pointer, if one is.
    pub fn hands_off(&self) -> Option<&Task> {
        self.tasks
            .iter()
            .find(|task| task.hands_off && task.phase.live())
    }

    /// Until when the desk frame shows: only while the agent has the
    /// person's controls, a live task moving the real pointer or keys
    /// ("Hands off") or a window it brought in front (a borrow); held
    /// [`ACTING_FOR`] ahead, and the next event decides again. Work in
    /// parked windows, hidden browsers, or files shows no frame, however
    /// busy (`docs/ux/redesign.md` §3.5). `None`: no frame.
    pub fn frame_until(&self, now: Instant) -> Option<Instant> {
        self.tasks
            .iter()
            .any(|task| task.phase.live() && (task.hands_off || task.borrowed.is_some()))
            .then_some(now + ACTING_FOR)
    }

    /// The live task whose window is in front of the person now, for the
    /// desk frame's tab (the first, should two be).
    pub fn borrowing(&self) -> Option<&Task> {
        self.tasks
            .iter()
            .find(|task| task.phase.live() && task.borrowed.is_some())
    }

    /// The person's app in front again after a borrow, while "Back to you"
    /// shows ([`BACK_FOR`]).
    pub fn back_to(&self, now: Instant) -> Option<&str> {
        if self.borrowing().is_some() {
            return None;
        }
        self.tasks
            .iter()
            .filter_map(|task| task.returned.as_ref())
            .filter(|(_, at)| now < *at + BACK_FOR)
            .max_by_key(|(_, at)| *at)
            .map(|(app, _)| app.as_str())
    }

    /// The live tasks waiting on a question for the person, newest first.
    pub fn questions(&self) -> Vec<&Task> {
        let mut asking: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|task| task.phase.live() && task.question.is_some())
            .collect();
        asking.sort_by_key(|task| std::cmp::Reverse(task.asked));
        asking
    }

    /// Every question waiting on the person, the active one included.
    pub fn open_questions(&self) -> usize {
        match &self.inbox {
            Some(inbox) => inbox.waiting + usize::from(inbox.active.is_some()),
            None => self.questions().len(),
        }
    }

    /// The inbox's active question's conversation, once told.
    pub fn active_id(&self) -> Option<&str> {
        self.inbox
            .as_ref()
            .and_then(|inbox| inbox.active.as_ref())
            .map(|active| active.conversation.as_str())
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// `id` while it runs, is queued, or is held.
    pub fn live_task(&self, id: &str) -> Option<&Task> {
        self.task(id).filter(|task| task.phase.live())
    }

    /// Why live task `id` isn't moving, as its events said.
    pub fn hold(&self, id: &str) -> Option<Hold> {
        self.live_task(id).and_then(Task::hold)
    }

    /// Runs `change` on a live task named `id`; events for a task that's
    /// over or unknown change nothing.
    fn live(&mut self, id: &str, change: impl FnOnce(&mut Task) -> bool) -> bool {
        match self.task_mut(id) {
            Some(task) if task.phase.live() => change(task),
            _ => false,
        }
    }

    fn finish(
        &mut self,
        id: &str,
        phase: Phase,
        now: Instant,
        change: impl FnOnce(&mut Task),
    ) -> bool {
        let Some(task) = self.task_mut(id) else {
            return false;
        };
        task.phase = phase;
        task.ended = Some(now);
        task.step = None;
        task.hands_off = false;
        task.borrowed = None;
        change(task);
        true
    }

    /// The lowest colour no live task wears, for a task the service gave
    /// none (so a fixture or a late subscriber still tells tasks apart).
    fn free_colour(&self) -> u8 {
        (0..COLOURS)
            .find(|colour| {
                !self
                    .tasks
                    .iter()
                    .any(|task| task.phase.live() && task.colour == *colour)
            })
            .unwrap_or(self.tasks.len() as u8 % COLOURS)
    }
}

/// Sets `slot` to `value`; whether it changed.
fn set<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}
