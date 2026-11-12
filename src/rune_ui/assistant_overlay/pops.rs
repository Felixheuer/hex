//! The small bubbles that pop out of the orb about Background tasks, so
//! their progress shows for a moment and then gets out of the way: a new
//! task's title, its latest step as a brief toast, ✓ with the answer's first
//! line when it's done, ✗ with why when it failed. No list stays up; the
//! expanded list (double-tap, or hover the orb) has the rest.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Event};

use super::board::Task;

/// A new task's title stays this long.
const NEW: Duration = Duration::from_millis(2600);
/// A step's toast; a later step of the same task replaces its text.
const STEP: Duration = Duration::from_millis(2400);
/// ✓ and the answer's first line, then it dissolves.
const DONE: Duration = Duration::from_secs(4);
/// ✗ and why, as long as a failure notice.
const FAILED: Duration = Duration::from_secs(6);
/// Bubbles up at once; an older one makes way.
pub(crate) const MAX_POPS: usize = 3;

/// What a bubble says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopKind {
    /// A task just started (or queued): its title.
    New,
    /// What it's doing now.
    Step,
    /// ✓ It's done.
    Done,
    /// ✗ It failed.
    Failed,
}

/// One bubble.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pop {
    /// Its own, so a bubble that leaves animates out and a new one in.
    pub(crate) key: u64,
    pub(crate) task: ConversationId,
    pub(crate) colour: u8,
    pub(crate) kind: PopKind,
    pub(crate) text: String,
    leaves: Instant,
}

/// The bubbles up now, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pops {
    pub(crate) items: Vec<Pop>,
    next: u64,
}

impl Pops {
    /// The bubbles after `event` about `task` (as it is after the event);
    /// `fresh` when the task wasn't live before it (a new task, or a
    /// follow-up). Whether they changed.
    pub(crate) fn apply(&mut self, event: &Event, task: &Task, fresh: bool, now: Instant) -> bool {
        match event {
            // A new task, or a Here question that became one.
            Event::Escalated { .. } => {
                self.push(task, PopKind::New, task.title.clone(), now + NEW);
                true
            }
            Event::Queued { .. } | Event::Started { .. } if fresh => {
                self.clear(&task.id);
                self.push(task, PopKind::New, task.title.clone(), now + NEW);
                true
            }
            Event::Step { .. } => {
                let Some(text) = task.step.clone() else {
                    return false;
                };
                if let Some(pop) = self
                    .items
                    .iter_mut()
                    .find(|pop| pop.task == task.id && pop.kind == PopKind::Step)
                {
                    pop.text = text;
                    pop.leaves = now + STEP;
                } else {
                    self.push(task, PopKind::Step, text, now + STEP);
                }
                true
            }
            Event::Answer { .. } => {
                self.clear(&task.id);
                let text = task.detail().unwrap_or_else(|| task.title.clone());
                self.push(task, PopKind::Done, text, now + DONE);
                true
            }
            Event::Failed { .. } => {
                self.clear(&task.id);
                let text = task.detail().unwrap_or_else(|| task.title.clone());
                self.push(task, PopKind::Failed, text, now + FAILED);
                true
            }
            Event::Cancelled { .. } => self.clear(&task.id),
            _ => false,
        }
    }

    fn push(&mut self, task: &Task, kind: PopKind, text: String, leaves: Instant) {
        self.next += 1;
        self.items.push(Pop {
            key: self.next,
            task: task.id.clone(),
            colour: task.colour,
            kind,
            text,
            leaves,
        });
        let extra = self.items.len().saturating_sub(MAX_POPS);
        self.items.drain(..extra);
    }

    /// Takes down `task`'s bubbles; whether any were up.
    fn clear(&mut self, task: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|pop| pop.task != task);
        self.items.len() != before
    }

    /// Takes down bubbles whose time is up; whether any went.
    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|pop| now < pop.leaves);
        self.items.len() != before
    }

    /// When the next bubble leaves.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.items.iter().map(|pop| pop.leaves).min()
    }
}

#[cfg(test)]
mod tests {
    use super::super::board::Board;
    use super::*;
    use crate::assistant::Origin;

    fn started(id: &str, origin: Origin) -> Event {
        Event::Started {
            conversation: id.into(),
            origin,
            question: format!("Task {id}"),
        }
    }

    fn step(id: &str, text: &str) -> Event {
        Event::Step {
            conversation: id.into(),
            description: text.into(),
        }
    }

    fn kinds(board: &Board) -> Vec<(String, PopKind)> {
        board
            .pops
            .items
            .iter()
            .map(|pop| (pop.task.clone(), pop.kind))
            .collect()
    }

    #[test]
    fn a_task_pops_its_title_then_steps_as_one_toast_then_a_tick() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(
            &Event::Queued {
                conversation: "a".into(),
                origin: Origin::Background,
                question: "Book a table".into(),
            },
            None,
            t0,
        );
        board.apply(&started("a", Origin::Background), None, t0);
        assert_eq!(
            kinds(&board),
            [("a".into(), PopKind::New)],
            "Started after Queued pops once"
        );
        board.apply(&step("a", "Opened Safari"), None, t0);
        board.apply(&step("a", "Typed Nopa"), None, t0 + Duration::from_secs(1));
        assert_eq!(
            kinds(&board),
            [("a".into(), PopKind::New), ("a".into(), PopKind::Step)],
            "a later step replaces the toast's text"
        );
        assert_eq!(board.pops.items[1].text, "Typed Nopa");
        assert!(board.expire(t0 + NEW));
        assert_eq!(kinds(&board), [("a".into(), PopKind::Step)]);
        board.apply(
            &Event::Answer {
                conversation: "a".into(),
                origin: Origin::Background,
                text: "Booked Nopa for 8 pm.".into(),
            },
            None,
            t0 + Duration::from_secs(2),
        );
        assert_eq!(kinds(&board), [("a".into(), PopKind::Done)]);
        assert_eq!(board.pops.items[0].text, "Booked Nopa for 8 pm.");
        let gone = t0 + Duration::from_secs(2) + DONE;
        assert_eq!(board.pops.next_expiry(), Some(gone));
        assert!(board.expire(gone));
        assert!(board.pops.items.is_empty());
    }

    #[test]
    fn here_questions_never_pop_and_old_bubbles_make_way() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started("q", Origin::ActionShortcut), None, t0);
        board.apply(&step("q", "Read the window"), None, t0);
        assert!(board.pops.items.is_empty());
        for id in ["a", "b", "c", "d"] {
            board.apply(&started(id, Origin::Background), None, t0);
        }
        let tasks: Vec<_> = board.pops.items.iter().map(|p| p.task.as_str()).collect();
        assert_eq!(tasks, ["b", "c", "d"]);
        board.apply(
            &Event::Cancelled {
                conversation: "c".into(),
            },
            None,
            t0,
        );
        let tasks: Vec<_> = board.pops.items.iter().map(|p| p.task.as_str()).collect();
        assert_eq!(tasks, ["b", "d"], "a stopped task's bubbles go");
    }
}
