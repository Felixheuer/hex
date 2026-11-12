//! Which conversation Chats shows without being told: opening Chats or
//! bringing its window forward lands on the most relevant one, and while
//! the person hasn't navigated away Chats follows new activity there. Once
//! they pick a conversation themselves it stays put, and new activity
//! elsewhere offers "Jump to current". The transcript keeps to its end
//! while it streams unless they scrolled up, which offers "↓ New".

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ChatsPane;
use crate::assistant::{ConversationId, Origin, TaskState};

/// How long typing or scrolling up holds following back.
pub(super) const QUIET: Duration = Duration::from_secs(10);

/// The transcript viewport's key, for its scroll.
pub(super) const SCROLL: &str = "chat-scroll";

/// Where the transcript is scrolled, as its viewport last reported it.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::rune_ui) struct ScrollWatch {
    /// Scrolled up from the end.
    pub(super) up: bool,
    /// When it last moved up from the end.
    pub(super) up_at: Option<Instant>,
}

#[derive(Default)]
pub(in crate::rune_ui) struct Follow {
    /// The person picked the shown conversation themselves.
    pub(super) pinned: bool,
    /// When they last typed in the composer.
    pub(super) typed_at: Option<Instant>,
    /// The composer's text as last seen, to notice typing.
    typed: String,
    /// New activity elsewhere while not following: "Jump to current".
    pub(super) elsewhere: Option<ConversationId>,
    /// The transcript got more while scrolled up: "↓ New".
    pub(super) unseen: bool,
    /// The transcript should go to its end on the next frame (a switch).
    pub(super) to_end: bool,
    /// The pane showed at the last poll.
    pub(super) was_showing: bool,
    /// `show` named where to land; the next appearance keeps it.
    pub(super) landed: bool,
    pub(super) scroll: Arc<Mutex<ScrollWatch>>,
}

impl Follow {
    pub(super) fn scroll(&self) -> ScrollWatch {
        self.scroll.lock().map(|watch| *watch).unwrap_or_default()
    }
}

impl ChatsPane {
    /// The conversation most worth showing: a running Here task, else the
    /// most recently started running task, else the latest conversation.
    pub(super) fn most_relevant(&self) -> Option<ConversationId> {
        let here = self
            .tasks
            .iter()
            .rev()
            .find(|task| task.origin == Origin::ActionShortcut)
            .map(|task| task.conversation.clone())
            .or_else(|| {
                self.turns
                    .iter()
                    .find(|(_, turn)| turn.origin == Origin::ActionShortcut)
                    .map(|(id, _)| id.clone())
            });
        if here.is_some() {
            return here;
        }
        // Oldest first: the last one started is the most recent.
        let started = self
            .tasks
            .iter()
            .rev()
            .find(|task| task.state != TaskState::Queued)
            .or_else(|| self.tasks.last())
            .map(|task| task.conversation.clone());
        started
            .or_else(|| {
                let mut turns: Vec<&ConversationId> = self.turns.keys().collect();
                turns.sort();
                turns.last().map(|id| (*id).clone())
            })
            .or_else(|| self.list.first().map(|row| row.id.clone()))
    }

    /// Lands on the most relevant conversation and follows from there.
    pub(super) fn auto_select(&mut self) {
        self.follow.pinned = false;
        self.follow.elsewhere = None;
        if let Some(id) = self.most_relevant() {
            self.switch_to(id);
        }
    }

    /// The person picked `id`: Chats stops following.
    pub(super) fn pick(&mut self, id: ConversationId) {
        self.follow.pinned = true;
        if self.follow.elsewhere.as_ref() == Some(&id) {
            self.follow.elsewhere = None;
        }
        self.select(id);
    }

    /// "Jump to current": the activity elsewhere, following again.
    pub(super) fn jump_to_current(&mut self) {
        let Some(id) = self.follow.elsewhere.take() else {
            return;
        };
        self.follow.pinned = false;
        self.follow.typed_at = None;
        self.switch_to(id);
    }

    /// Lands on `id` (a link, the HUD, the task stack) and follows from there.
    pub(super) fn land(&mut self, id: ConversationId) {
        self.follow.pinned = false;
        self.follow.elsewhere = None;
        self.switch_to(id);
    }

    /// Shows `id` without pinning it, its end in view.
    fn switch_to(&mut self, id: ConversationId) {
        if self.selected.as_ref() != Some(&id) {
            self.follow.to_end = true;
            self.follow.unseen = false;
        }
        self.select(id);
    }

    /// Whether Chats follows new activity now: nothing picked, and no
    /// typing or scrolling up within [`QUIET`].
    pub(super) fn following(&self, now: Instant) -> bool {
        let recent = |at: Option<Instant>| at.is_some_and(|at| now.duration_since(at) < QUIET);
        let scroll = self.follow.scroll();
        !self.follow.pinned && !recent(self.follow.typed_at) && !scroll.up && !recent(scroll.up_at)
    }

    /// A task started or a question was asked in `id`: follow it there, or
    /// offer to jump to it.
    pub(super) fn activity(&mut self, id: &ConversationId, now: Instant) {
        if self.selected.as_ref() == Some(id) {
            return;
        }
        if self.following(now) {
            self.follow.elsewhere = None;
            self.switch_to(id.clone());
        } else {
            self.follow.elsewhere = Some(id.clone());
        }
    }

    /// More arrived in conversation `id`: "↓ New" while scrolled up.
    pub(super) fn more_in(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) && self.follow.scroll().up {
            self.follow.unseen = true;
        }
    }

    /// Notices typing in the composer, and the end of the transcript seen.
    pub(super) fn watch(&mut self, now: Instant) {
        if self.composer.text != self.follow.typed {
            self.follow.typed.clone_from(&self.composer.text);
            if !self.composer.text.is_empty() {
                self.follow.typed_at = Some(now);
            }
        }
        if !self.follow.scroll().up {
            self.follow.unseen = false;
        }
        if self
            .follow
            .elsewhere
            .as_ref()
            .is_some_and(|id| !self.running(id))
        {
            self.follow.elsewhere = None;
        }
    }
}

/// The transcript viewport's scroll handler: notes whether it's up from
/// its end.
pub(super) fn on_scroll(watch: Arc<Mutex<ScrollWatch>>) -> impl Fn(rune::ScrollPos) + 'static {
    move |at: rune::ScrollPos| {
        let up = at.max[1] > 1.0 && at.offset[1] < at.max[1] - 24.0;
        if let Ok(mut watch) = watch.lock() {
            if up && !watch.up {
                watch.up_at = Some(Instant::now());
            }
            watch.up = up;
        }
    }
}
