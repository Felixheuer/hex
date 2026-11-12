//! The Try page's tutorial: the Action shortcut's three gestures, each
//! checked live. Hold asks about the window in front (done once its answer
//! comes back), tap then hold starts a Background task (done once it's
//! queued or started), and double-tap opens the task list, which only the
//! HUD sees: it reports it through [`tasks_opened`], and the step is
//! optional.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::assistant::{ConversationId, Event, Origin};

/// How many times the HUD's task list opened by a double-tap.
static TASKS_OPENED: AtomicU64 = AtomicU64::new(0);

/// The HUD's expanded task list opened from a double-tap of the Action
/// shortcut. The overlay calls it; the tutorial's third step watches it.
pub(crate) fn tasks_opened() {
    TASKS_OPENED.fetch_add(1, Ordering::Relaxed);
}

/// The Here gesture's question: heard, then answered.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::rune_ui) struct Trial {
    pub(in crate::rune_ui) asked: Option<ConversationId>,
    pub(in crate::rune_ui) heard: Option<String>,
    pub(in crate::rune_ui) answer: Option<Result<String, String>>,
}

impl Trial {
    pub(in crate::rune_ui) fn answered(&self) -> bool {
        matches!(self.answer, Some(Ok(_)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) struct Tutorial {
    /// Hold: a question about the window in front.
    pub(in crate::rune_ui) here: Trial,
    /// Tap, then hold: the Background task's request, once it started.
    pub(in crate::rune_ui) background: Option<String>,
    /// Double-tap: the task list opened (optional).
    pub(in crate::rune_ui) tasks: bool,
    /// [`TASKS_OPENED`] when the page opened.
    opened_before: u64,
}

impl Default for Tutorial {
    fn default() -> Self {
        Self {
            here: Trial::default(),
            background: None,
            tasks: false,
            opened_before: TASKS_OPENED.load(Ordering::Relaxed),
        }
    }
}

/// The three gestures, in the order the page teaches them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Gesture {
    Here,
    Background,
    Tasks,
}

impl Tutorial {
    /// Both required gestures worked.
    pub(in crate::rune_ui) fn done(&self) -> bool {
        self.here.answered() && self.background.is_some()
    }

    /// Whether `gesture` was seen working.
    pub(in crate::rune_ui) fn worked(&self, gesture: Gesture) -> bool {
        match gesture {
            Gesture::Here => self.here.answered(),
            Gesture::Background => self.background.is_some(),
            Gesture::Tasks => self.tasks,
        }
    }

    /// The gesture to try next.
    pub(in crate::rune_ui) fn next(&self) -> Option<Gesture> {
        [Gesture::Here, Gesture::Background, Gesture::Tasks]
            .into_iter()
            .find(|gesture| !self.worked(*gesture))
    }

    /// Starts the Here gesture over.
    pub(in crate::rune_ui) fn retry_here(&mut self) {
        self.here = Trial::default();
    }

    /// Notices a double-tap the HUD reported. True when it was new.
    pub(in crate::rune_ui) fn poll(&mut self) -> bool {
        if self.tasks {
            return false;
        }
        self.tasks = TASKS_OPENED.load(Ordering::Relaxed) > self.opened_before;
        self.tasks
    }

    /// Follows the gestures' requests. True when it changed.
    pub(in crate::rune_ui) fn event(&mut self, event: &Event) -> bool {
        let here = &mut self.here;
        match event {
            Event::Queued {
                origin: Origin::Background,
                question,
                ..
            }
            | Event::Started {
                origin: Origin::Background,
                question,
                ..
            } if self.background.is_none() => {
                self.background = Some(question.clone());
            }
            Event::Queued {
                conversation,
                origin: Origin::ActionShortcut,
                question,
            }
            | Event::Started {
                conversation,
                origin: Origin::ActionShortcut,
                question,
            } if !here.answered() => {
                here.asked = Some(conversation.clone());
                here.heard = Some(question.clone());
                here.answer = None;
            }
            Event::Answer {
                conversation,
                origin: Origin::ActionShortcut,
                text,
            } if here.asked.as_ref().is_none_or(|asked| asked == conversation)
                && !here.answered() =>
            {
                here.asked = None;
                here.answer = Some(Ok(text.trim().to_owned()));
            }
            Event::Failed {
                conversation,
                message,
            } if here.asked.as_ref() == Some(conversation) => {
                here.asked = None;
                here.answer = Some(Err(message.clone()));
            }
            Event::Cancelled { conversation } if here.asked.as_ref() == Some(conversation) => {
                here.asked = None;
                here.heard = None;
            }
            _ => return false,
        }
        true
    }

    /// Every gesture worked, for scenes.
    pub(in crate::rune_ui) fn fixture(all: bool) -> Self {
        Self {
            here: Trial {
                asked: None,
                heard: Some("What's on this page?".into()),
                answer: Some(Ok(
                    "Hex's release notes: version 0.32 adds Background tasks.".into()
                )),
            },
            background: all.then(|| "In the background, find a free hour next week".into()),
            tasks: false,
            opened_before: 0,
        }
    }
}
