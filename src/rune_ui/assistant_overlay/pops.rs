//! The bubbles that pop out of the orb when a job ends, so its result shows
//! for a moment and then gets out of the way: ✓ with the answer's first
//! lines when it's done, ✗ with why when it failed ([`super::results`]
//! draws them). A job's start and steps don't pop: its row at the orb
//! says them, so bubbles never pile up while jobs work. Unfolded (More) a
//! bubble stays until it's folded or closed; the orb's list recalls a
//! result after its bubble left.

use std::time::{Duration, Instant};

use crate::assistant::{ConversationId, Event};

use super::board::Task;

/// ✓ and the answer's first lines, then it dissolves: long enough to read
/// and press what it offers.
pub(crate) const DONE: Duration = Duration::from_secs(12);
/// ✗ and why: long enough to be seen by someone who looked away, since a
/// job that failed unnoticed costs hours.
pub(crate) const FAILED: Duration = Duration::from_secs(10 * 60);
/// An unfolded bubble stays this long at most, should it be forgotten.
const HELD: Duration = Duration::from_secs(60 * 60);
/// Bubbles up at once; an older one makes way (into the list's Done).
pub(crate) const MAX_POPS: usize = 2;

/// What a bubble says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopKind {
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
    /// The answer's first line, or why it failed.
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
            // Running again (Try again, an answer, a follow-up): its old
            // result is moot.
            Event::Queued { .. } | Event::Started { .. } if fresh => self.clear(&task.id),
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
        // Too many: the oldest make way, failures last.
        while self.items.len() > MAX_POPS {
            let oldest = self
                .items
                .iter()
                .position(|pop| pop.kind != PopKind::Failed)
                .unwrap_or(0);
            self.items.remove(oldest);
        }
    }

    /// `task`'s result as its bubble again (a press in the list), held
    /// unfolded; the bubble's key.
    pub(crate) fn recall(&mut self, task: &Task, failed: bool, text: String, now: Instant) -> u64 {
        self.clear(&task.id);
        let kind = if failed {
            PopKind::Failed
        } else {
            PopKind::Done
        };
        self.push(task, kind, text, now + HELD);
        self.next
    }

    /// Keeps bubble `key` up (unfolded), or lets it go `after` from `now`
    /// (folded again); whether it's up.
    pub(crate) fn hold(&mut self, key: u64, held: bool, now: Instant) -> bool {
        let Some(pop) = self.items.iter_mut().find(|pop| pop.key == key) else {
            return false;
        };
        pop.leaves = now
            + if held {
                HELD
            } else if pop.kind == PopKind::Failed {
                FAILED
            } else {
                DONE
            };
        true
    }

    /// Takes bubble `key` down (×); whether it was up.
    pub(crate) fn dismiss(&mut self, key: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|pop| pop.key != key);
        self.items.len() != before
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
    fn a_job_pops_nothing_while_it_works_then_its_result() {
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
        board.apply(&step("a", "Opened Safari"), None, t0);
        assert!(kinds(&board).is_empty(), "its row says it, no toast");
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
        // Running again takes an old result down.
        board.apply(&started("a", Origin::Background), None, gone);
        board.apply(
            &Event::Failed {
                conversation: "a".into(),
                message: "No table left".into(),
            },
            None,
            t0,
        );
        assert_eq!(kinds(&board), [("a".into(), PopKind::Failed)]);
        board.apply(&started("a", Origin::Background), None, t0);
        assert!(kinds(&board).is_empty(), "Try again: the failure goes");
    }

    #[test]
    fn a_failure_stays_and_is_the_last_to_make_way() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started("x", Origin::Background), None, t0);
        board.apply(
            &Event::Failed {
                conversation: "x".into(),
                message: "The site refused the login".into(),
            },
            None,
            t0,
        );
        for id in ["a", "b", "c"] {
            board.apply(&started(id, Origin::Background), None, t0);
            board.apply(
                &Event::Answer {
                    conversation: id.into(),
                    origin: Origin::Background,
                    text: "Done.".into(),
                },
                None,
                t0,
            );
        }
        assert_eq!(
            kinds(&board),
            [("x".into(), PopKind::Failed), ("c".into(), PopKind::Done)],
            "results don't push a failure out"
        );
        assert!(!board.expire(t0 + Duration::from_secs(1)));
        assert!(board.orb_shows(t0), "the orb stays while it shows");
    }

    #[test]
    fn here_questions_never_pop_and_a_stopped_job_s_result_goes() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started("q", Origin::ActionShortcut), None, t0);
        board.apply(
            &Event::Answer {
                conversation: "q".into(),
                origin: Origin::ActionShortcut,
                text: "Sunny".into(),
            },
            None,
            t0,
        );
        assert!(board.pops.items.is_empty());
        board.apply(&started("a", Origin::Background), None, t0);
        board.apply(
            &Event::Cancelled {
                conversation: "a".into(),
            },
            None,
            t0,
        );
        assert!(board.pops.items.is_empty());
    }

    #[test]
    fn an_unfolded_bubble_stays_and_a_closed_one_goes() {
        let t0 = Instant::now();
        let mut board = Board::default();
        board.apply(&started("a", Origin::Background), None, t0);
        board.apply(
            &Event::Answer {
                conversation: "a".into(),
                origin: Origin::Background,
                text: "Done.".into(),
            },
            None,
            t0,
        );
        let key = board.pops.items[0].key;
        assert!(board.pops.hold(key, true, t0));
        assert!(!board.pops.expire(t0 + DONE * 2), "unfolded, it stays");
        assert!(board.pops.hold(key, false, t0));
        assert!(board.pops.expire(t0 + DONE));
        assert!(!board.pops.dismiss(key));
        board.apply(&started("a", Origin::Background), None, t0);
        board.apply(
            &Event::Failed {
                conversation: "a".into(),
                message: "x".into(),
            },
            None,
            t0,
        );
        let key = board.pops.items[0].key;
        assert!(board.pops.dismiss(key));
        assert!(board.pops.items.is_empty());
    }
}
