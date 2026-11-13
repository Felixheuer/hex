//! What the assistant's overlays show, over the shared task view
//! (`assistant::view`, which the Assistant pane reads too): the tasks (for
//! the orb, its list, and the window outlines), the bubbles that pop from
//! the orb ([`Pops`]), the results it recalls, and which question's bubble
//! the person put away. No drawing and no platform here, so every rule is a
//! unit test.

use std::ops::{Deref, DerefMut};
use std::time::Instant;

use crate::assistant::view::{Inbox, Stream};
pub(crate) use crate::assistant::view::{Phase, Place, Task, first_line, plain, shorten};
use crate::assistant::{ConversationId, Event, Origin, TaskStatus, TaskWindow};

use super::pops::Pops;
pub(crate) use super::quick::{Bubble, BubblePhase, Due};
use super::results::{Outcome, RECALL_FOR, RECENT_MAX};

/// How a window's outline looks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Look {
    /// Its task works there: a solid line in its colour.
    #[default]
    Working,
    /// The person has it: a dashed line and "You have it".
    Paused,
    /// Waiting out a usage limit: dimmed, with when it resumes.
    Waiting { until_ms: u64 },
    /// A task moves the real pointer there: a soft line.
    HandsOff,
}

/// One window a live task works in, for its outline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Outlined {
    pub(crate) task: ConversationId,
    pub(crate) colour: u8,
    pub(crate) title: String,
    pub(crate) window: TaskWindow,
    pub(crate) look: Look,
    /// Its plan's steps done and in all, for the title chip.
    pub(crate) progress: Option<(usize, usize)>,
    /// What it does there now, for the title chip.
    pub(crate) step: Option<String>,
    /// A Here task's window: outlined in the agent's hue.
    pub(crate) here: bool,
}

/// The question the orb's bubble shows: the inbox's active one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asking {
    pub(crate) id: ConversationId,
    /// Its task's title, for "… asks".
    pub(crate) title: String,
    pub(crate) question: String,
    pub(crate) colour: u8,
    /// The project its task runs in, by name: the card says so.
    pub(crate) project: Option<String>,
}

/// Every task the overlays know of (the shared view, which `Board`
/// derefs to), and the bubbles that pop from the orb about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Board {
    pub(crate) view: Stream,
    pub(crate) pops: Pops,
    /// The active question whose bubble the person put away (×); it comes
    /// back when another becomes active.
    hidden: Option<ConversationId>,
    /// The jobs that ended lately, oldest first: the orb recalls them for
    /// [`RECALL_FOR`] ([`super::results`]).
    pub(crate) recent: Vec<Outcome>,
}

impl Deref for Board {
    type Target = Stream;

    fn deref(&self) -> &Stream {
        &self.view
    }
}

impl DerefMut for Board {
    fn deref_mut(&mut self) -> &mut Stream {
        &mut self.view
    }
}

impl Board {
    /// The tasks running or queued when the overlays started listening.
    pub(crate) fn seeded(statuses: Vec<TaskStatus>) -> Self {
        Self {
            view: Stream::seeded(&statuses),
            ..Self::default()
        }
    }

    /// Applies `event`; `colour` is the service's colour for a task it
    /// starts (`TaskStatus.colour`), when it has one. Whether anything the
    /// overlays draw changed.
    pub(crate) fn apply(&mut self, event: &Event, colour: Option<u8>, now: Instant) -> bool {
        let fresh = event
            .conversation()
            .is_none_or(|id| self.task(id).is_none_or(|task| !task.phase.live()));
        let active = self.active_id().map(str::to_owned);
        let changed = self.view.apply(event, colour, now);
        if self.active_id() != active.as_deref() {
            // Another question is active: a put-away bubble comes back.
            self.hidden = None;
        }
        // Running again (Try again, an answer, a follow-up), however it was
        // asked: its old result is moot.
        let restarted = match event {
            Event::Queued { conversation, .. } | Event::Started { conversation, .. } => {
                let before = self.recent.len();
                self.recent.retain(|outcome| &outcome.id != conversation);
                self.recent.len() != before
            }
            _ => false,
        };
        // Jobs' results; a team's helpers stay quiet (their lead says it).
        let popped = match event.conversation().and_then(|id| self.task(id)) {
            Some(task) if task.origin == Origin::Background && task.lead.is_none() => {
                let task = task.clone();
                let noted = self.note_outcome(event, &task, now);
                self.pops.apply(event, &task, fresh, now) || noted
            }
            _ => false,
        };
        changed || popped || restarted
    }

    /// Keeps a job's result for the orb to recall; whether it did.
    fn note_outcome(&mut self, event: &Event, task: &Task, now: Instant) -> bool {
        match event {
            Event::Answer { .. } | Event::Failed { .. } => {
                self.recent.retain(|outcome| outcome.id != task.id);
                self.recent.push(Outcome::of(
                    task,
                    matches!(event, Event::Failed { .. }),
                    now,
                ));
                let extra = self.recent.len().saturating_sub(RECENT_MAX);
                self.recent.drain(..extra);
                true
            }
            _ => false,
        }
    }

    /// The results the orb recalls, newest first.
    pub(crate) fn results(&self) -> Vec<&Outcome> {
        self.recent.iter().rev().collect()
    }

    /// `id`'s result.
    pub(crate) fn outcome(&self, id: &str) -> Option<&Outcome> {
        self.recent.iter().find(|outcome| outcome.id == id)
    }

    /// `id`'s result as its bubble again, unfolded (a press in the list);
    /// the bubble's key.
    pub(crate) fn recall(&mut self, id: &str, now: Instant) -> Option<u64> {
        let outcome = self.outcome(id)?.clone();
        // Recalled, it counts as new: it stays its full time again.
        if let Some(kept) = self.recent.iter_mut().find(|kept| kept.id == id) {
            kept.at = now;
        }
        let task = Task::new(
            outcome.id.clone(),
            Origin::Background,
            &outcome.title,
            outcome.colour,
            if outcome.failed {
                Phase::Failed
            } else {
                Phase::Done
            },
        );
        Some(
            self.pops
                .recall(&task, outcome.failed, outcome.summary(), now),
        )
    }

    /// Drops finished tasks, bubbles, and results whose time is up;
    /// whether any went.
    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let before = self.recent.len();
        let tasks = self.view.expire(now);
        self.recent.retain(|outcome| now < outcome.at + RECALL_FOR);
        let popped = self.pops.expire(now);
        tasks || self.recent.len() != before || popped
    }

    /// When the next finished task, bubble, or result leaves.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.view
            .next_expiry()
            .into_iter()
            .chain(self.pops.next_expiry())
            .chain(self.recent.iter().map(|outcome| outcome.at + RECALL_FOR))
            .min()
    }

    /// Whether the orb has anything to show at `now`: a Background task, a
    /// bubble about one, or a result it can still recall.
    pub(crate) fn orb_shows(&self, now: Instant) -> bool {
        !self.pops.items.is_empty()
            || self
                .tasks
                .iter()
                .any(|task| task.origin == Origin::Background)
            || self
                .recent
                .iter()
                .any(|outcome| now < outcome.at + RECALL_FOR)
    }

    /// Every live task's windows, whatever started it, for their outlines.
    pub(crate) fn outlined(&self) -> Vec<Outlined> {
        self.tasks
            .iter()
            .filter(|task| task.phase.live())
            .flat_map(|task| {
                let look = match task.phase {
                    Phase::Paused => Look::Paused,
                    Phase::Waiting { until_ms } => Look::Waiting { until_ms },
                    _ if task.hands_off => Look::HandsOff,
                    _ => Look::Working,
                };
                task.windows.iter().map(move |window| Outlined {
                    task: task.id.clone(),
                    colour: task.colour,
                    title: task.title.clone(),
                    window: window.clone(),
                    look,
                    progress: task.progress(),
                    step: task.step.clone(),
                    here: task.origin.lane() == crate::assistant::Lane::Here,
                })
            })
            .collect()
    }

    /// Puts away `id`'s question bubble (it still waits for the answer);
    /// whether one was up.
    pub(crate) fn dismiss_question(&mut self, id: &str) -> bool {
        if self.inbox.is_some() {
            let active = self.active_question().is_some_and(|asking| asking.id == id);
            if active {
                self.hidden = Some(id.to_owned());
            }
            return active;
        }
        self.task_mut(id)
            .is_some_and(|task| task.question.take().is_some())
    }

    /// The question the orb's bubble shows: the inbox's active one (a
    /// Background task's), unless the person put it away; before the
    /// service told the inbox, the newest question.
    pub(crate) fn active_question(&self) -> Option<Asking> {
        let asking = match &self.inbox {
            Some(Inbox { active, .. }) => {
                let active = active.as_ref()?;
                let task = self.task(&active.conversation);
                if task.is_some_and(|task| task.origin != Origin::Background) {
                    return None;
                }
                Asking {
                    id: active.conversation.clone(),
                    title: task.map_or_else(|| shorten(&active.title), |task| task.title.clone()),
                    question: active.question.trim().to_owned(),
                    colour: task.map_or(0, |task| task.colour),
                    project: task.and_then(|task| task.project.clone()),
                }
            }
            None => {
                let task = self
                    .questions()
                    .into_iter()
                    .find(|task| task.origin == Origin::Background)?;
                Asking {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    question: task.question.clone().unwrap_or_default(),
                    colour: task.colour,
                    project: task.project.clone(),
                }
            }
        };
        (self.hidden.as_deref() != Some(asking.id.as_str())).then_some(asking)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::assistant::PlanStep;
    use crate::assistant::view::FINISHED_LINGER;

    fn id(n: u32) -> ConversationId {
        format!("c{n}")
    }

    fn started(n: u32, origin: Origin, question: &str) -> Event {
        Event::Started {
            conversation: id(n),
            origin,
            question: question.into(),
        }
    }

    fn window(app: &str) -> TaskWindow {
        TaskWindow {
            app: app.into(),
            window_id: None,
        }
    }

    #[test]
    fn a_task_runs_steps_and_lingers_after_its_answer() {
        let t0 = Instant::now();
        let mut board = Board::default();
        assert!(board.apply(
            &Event::Queued {
                conversation: id(1),
                origin: Origin::Background,
                question: "Book a table for two tonight".into(),
            },
            Some(3),
            t0,
        ));
        assert_eq!(board.tasks[0].phase, Phase::Queued);
        assert_eq!(board.tasks[0].colour, 3);
        board.apply(&started(1, Origin::Background, "Book a table"), None, t0);
        assert_eq!(
            board.tasks.len(),
            1,
            "Started after Queued is the same task"
        );
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert_eq!(board.tasks[0].colour, 3, "keeps its colour");
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Opened Safari".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].detail().as_deref(), Some("Opened Safari"));
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Safari"),
            },
            None,
            t0,
        );
        assert!(
            !board.apply(
                &Event::Window {
                    conversation: id(1),
                    window: window("Safari"),
                },
                None,
                t0,
            ),
            "the same window once"
        );
        assert_eq!(board.outlined().len(), 1);
        board.apply(
            &Event::Answer {
                conversation: id(1),
                origin: Origin::Background,
                text: "\nBooked Nopa for 8 pm.\nConfirmation sent to your email.".into(),
            },
            None,
            t0 + Duration::from_secs(10),
        );
        let task = &board.tasks[0];
        assert_eq!(task.phase, Phase::Done);
        assert_eq!(task.detail().as_deref(), Some("Booked Nopa for 8 pm…"));
        assert!(
            board.outlined().is_empty(),
            "a finished task's outline goes"
        );
        let ended = t0 + Duration::from_secs(10);
        let leaves = ended + FINISHED_LINGER;
        assert_eq!(board.next_expiry(), Some(leaves), "the task goes first");
        assert!(!board.expire(leaves - Duration::from_millis(1)));
        assert!(board.expire(leaves));
        assert!(board.tasks.is_empty());
        assert_eq!(board.pops.items.len(), 1, "its ✓ bubble stays a while");
        assert_eq!(board.results().len(), 1, "and its result longer");
        assert!(board.orb_shows(leaves));
        assert!(board.expire(ended + super::super::pops::DONE));
        assert!(board.pops.items.is_empty());
        let recalled = ended + super::super::results::RECALL_FOR;
        assert_eq!(board.next_expiry(), Some(recalled));
        assert!(board.orb_shows(recalled - Duration::from_secs(1)));
        assert!(board.expire(recalled));
        assert!(board.results().is_empty());
        assert!(!board.orb_shows(recalled), "nothing left: the orb goes");
    }

    #[test]
    fn a_result_is_recalled_unfolded_and_dropped_when_the_job_runs_again() {
        let t0 = Instant::now();
        let mut board = Board::default();
        for n in 1..=2 {
            board.apply(
                &started(n, Origin::Background, "Compare suppliers"),
                None,
                t0,
            );
        }
        board.apply(
            &Event::Answer {
                conversation: id(1),
                origin: Origin::Background,
                text: "Fertig.\n\nFiles: /tmp/Angebote".into(),
            },
            None,
            t0,
        );
        board.apply(
            &Event::Failed {
                conversation: id(2),
                message: "The shop refused the login.".into(),
            },
            None,
            t0,
        );
        let ids: Vec<_> = board.results().iter().map(|o| o.id.clone()).collect();
        assert_eq!(ids, [id(2), id(1)], "newest first");
        assert!(board.results()[0].failed);
        board.expire(t0 + super::super::pops::FAILED);
        assert!(board.pops.items.is_empty());
        let key = board.recall(&id(1), t0 + Duration::from_secs(1)).unwrap();
        assert_eq!(board.pops.items.len(), 1);
        assert_eq!(board.pops.items[0].key, key);
        assert!(board.recall(&id(9), t0).is_none());
        // Try again, after its task left the board: the failure's result
        // goes, and it's a job again.
        assert!(board.tasks.is_empty());
        board.apply(
            &started(2, Origin::Background, "Please try again"),
            None,
            t0,
        );
        assert!(board.outcome(&id(2)).is_none());
        assert_eq!(board.stack().len(), 1, "a job again");
        // However a follow-up is asked, the old result goes.
        board.apply(&started(1, Origin::Chat, "And the prices?"), None, t0);
        assert!(board.results().is_empty());
    }

    #[test]
    fn helpers_leave_no_result_of_their_own() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Helper"), None, t0);
        board.tasks[0].lead = Some(id(9));
        board.apply(
            &Event::Answer {
                conversation: id(1),
                origin: Origin::Background,
                text: "Done".into(),
            },
            None,
            t0,
        );
        assert!(board.results().is_empty());
        assert!(board.pops.items.is_empty());
    }

    #[test]
    fn the_stack_lists_jobs_and_outlines_cover_every_live_one() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "a"), None, t0);
        board.apply(&started(2, Origin::Chat, "b"), None, t0);
        board.apply(&started(3, Origin::ActionShortcut, "c"), None, t0);
        for n in 1..=3 {
            board.apply(
                &Event::Window {
                    conversation: id(n),
                    window: window("Notes"),
                },
                None,
                t0,
            );
        }
        let stack: Vec<_> = board.stack().into_iter().map(|t| t.id).collect();
        // Jobs only: a plain Chats reply (`Chat`) and a quick question
        // aren't; a Chats request that became a job was escalated to
        // Background.
        assert_eq!(stack, [id(1)]);
        assert_eq!(board.outlined().len(), 3);
    }

    fn quick(n: u32) -> Event {
        started(n, Origin::ActionShortcut, "What's the weather like?")
    }

    #[test]
    fn a_task_taken_over_waits_for_its_hand_back_and_its_outline_says_so() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Book a table"), None, t0);
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Safari"),
            },
            None,
            t0,
        );
        assert!(board.apply(
            &Event::Paused {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.tasks[0].phase, Phase::Paused);
        assert_eq!(board.outlined()[0].look, Look::Paused);
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Looked again".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::Paused, "only Resumed ends it");
        assert!(board.apply(
            &Event::Resumed {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert_eq!(board.outlined()[0].look, Look::Working);
        assert!(!board.apply(
            &Event::Resumed {
                conversation: id(1)
            },
            None,
            t0
        ));
    }

    #[test]
    fn a_task_waiting_out_a_limit_dims_until_it_runs_again() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started(1, Origin::Background, "Summarise"), None, t0);
        board.apply(
            &Event::Window {
                conversation: id(1),
                window: window("Notes"),
            },
            None,
            t0,
        );
        board.apply(
            &Event::Waiting {
                conversation: id(1),
                until_ms: 1_700_000_000_000,
                reason: "Claude's usage limit".into(),
            },
            None,
            t0,
        );
        assert!(board.waiting());
        assert_eq!(
            board.outlined()[0].look,
            Look::Waiting {
                until_ms: 1_700_000_000_000
            }
        );
        assert!(board.tasks[0].phase.live(), "still stoppable");
        board.apply(
            &Event::Thinking {
                conversation: id(1),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert!(!board.waiting());
    }

    #[test]
    fn questions_wait_for_an_answer_newest_first_and_plans_count_their_steps() {
        let t0 = Instant::now();
        let mut board = Board::default();
        for n in 1..=2 {
            board.apply(&started(n, Origin::Background, "a"), None, t0);
            board.apply(
                &Event::Question {
                    conversation: id(n),
                    text: format!("Question {n}? "),
                },
                None,
                t0,
            );
        }
        assert_eq!(board.tasks[0].phase, Phase::NeedsInput);
        let asking: Vec<_> = board.questions().iter().map(|t| t.id.clone()).collect();
        assert_eq!(asking, [id(2), id(1)], "newest first");
        assert_eq!(board.tasks[0].question.as_deref(), Some("Question 1?"));
        board.apply(
            &Event::Step {
                conversation: id(1),
                description: "Still looking".into(),
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].phase, Phase::NeedsInput, "until answered");
        assert!(board.dismiss_question(&id(2)));
        assert!(!board.dismiss_question(&id(2)));
        assert_eq!(board.tasks[1].phase, Phase::NeedsInput, "still waits");
        // The answer arrives as a follow-up in the same conversation.
        board.apply(&started(1, Origin::Background, "Finance"), None, t0);
        assert_eq!(board.tasks[0].phase, Phase::Running);
        assert!(board.tasks[0].question.is_none());

        let step = |title: &str, done| PlanStep {
            title: title.into(),
            done,
        };
        board.apply(
            &Event::Plan {
                conversation: id(1),
                steps: vec![step("a", true), step("b", false), step("c", false)],
            },
            None,
            t0,
        );
        assert_eq!(board.tasks[0].progress(), Some((1, 3)));
        assert_eq!(board.tasks[0].current_step(), Some(1));
    }

    #[test]
    fn an_escalated_here_question_joins_the_background_tasks() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&quick(1), None, t0);
        assert!(board.stack().is_empty());
        assert!(board.apply(
            &Event::Escalated {
                conversation: id(1)
            },
            None,
            t0
        ));
        assert_eq!(board.stack().len(), 1);
        assert!(board.pops.items.is_empty(), "its row says it; no toast");
        let mut bubble = None;
        Bubble::apply(&mut bubble, &quick(1), t0);
        Bubble::apply(
            &mut bubble,
            &Event::Escalated {
                conversation: id(1),
            },
            t0,
        );
        let b = bubble.as_mut().unwrap();
        assert!(b.escalated);
        assert_eq!(b.due(t0, false), Due::Leave, "it flies off at once");
    }
}
