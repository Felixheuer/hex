//! The Action shortcut's side that doesn't depend on the OS, which the
//! macOS, Windows, and Linux listeners share. Each listener matches its own
//! keys and owns its own capture; this decides what a press means and what
//! the transcript then asks.
//!
//! - **Hold** asks Here, with the window in front: the desktop is read once
//!   the hold is real ([`Asking::intend`]), and the transcript continues
//!   `assistant::focus`.
//! - **Tap, then hold** starts a new Background task, with no conversation
//!   and no screenshot.
//! - **Double tap** shows or hides the HUD's task list ([`toggle_tasks`]).
//! - Words that open with "in the background" go to the Background lane
//!   without those words (`question::spoken_background`).
//! - Escape stops a running Here task and its speech, never a Background
//!   one ([`stop_here`]); a listener takes Escape only while
//!   [`claims_escape`] says so. Stop all tasks stops everything
//!   ([`stop_all`]).
//!
//! macOS tells a tap from a hold in its own shortcut machine
//! (`suppression::DictationHotkey`), which also sorts out chords and lost
//! key releases. Windows and Linux, whose hooks and portals report a press
//! and its release, use [`Gesture`].

// Windows and Linux tell the gestures apart here; macOS in its own machine.
#[cfg(any(test, not(target_os = "macos")))]
mod gesture;
mod question;
#[cfg(test)]
mod tests;

use std::sync::Mutex;
#[cfg(not(target_os = "macos"))]
use std::sync::atomic::{AtomicBool, Ordering};

use crate::assistant::{ConversationId, Lane, Request, TaskStatus};
use crate::audio::CaptureInstant;
use crate::control::PrintReply;
use crate::feedback::{self, Tone};

#[cfg(any(test, not(target_os = "macos")))]
pub(crate) use gesture::{Gesture, GestureAction};
pub(crate) use question::Question;
#[cfg(test)]
use question::spoken_background;

/// What a gesture of the Action shortcut calls (set by its owner).
type Hook = Box<dyn Fn() + Send + Sync>;

static TOGGLE_TASKS: Mutex<Option<Hook>> = Mutex::new(None);

/// Sets what a double tap of the Action shortcut does: show or hide the
/// HUD's expanded task list. It is called on the listener's thread, so post
/// to your own and return at once. A later call replaces an earlier one;
/// until the first, a double tap only plays its tone.
pub(crate) fn set_toggle_tasks(toggle: impl Fn() + Send + Sync + 'static) {
    *TOGGLE_TASKS
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(Box::new(toggle));
}

pub(crate) fn toggle_tasks() {
    match TOGGLE_TASKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        Some(toggle) => toggle(),
        None => tracing::debug!("double tap: no task list to show yet"),
    }
}

/// What a hold of the Action shortcut calls while the assistant isn't set
/// up yet (`assistant::ready`).
static OPEN_SETUP: Mutex<Option<Hook>> = Mutex::new(None);

/// Sets what a hold of the Action shortcut does while the assistant's setup
/// isn't finished: open the onboarding at its saved page, and have the HUD
/// say "Finish setting up the assistant first". Nothing is recorded or
/// asked. It is called on the listener's thread, so post to your own and
/// return at once; until it is set, such a hold does nothing.
pub(crate) fn set_open_setup(open: impl Fn() + Send + Sync + 'static) {
    *OPEN_SETUP.lock().unwrap_or_else(|error| error.into_inner()) = Some(Box::new(open));
}

pub(crate) fn open_setup() {
    match OPEN_SETUP
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        Some(open) => open(),
        None => tracing::info!("Action shortcut held before the assistant's setup finished"),
    }
}

/// Whether a press of the Action shortcut from `pressed` to `released` was a
/// hold (past the hold threshold), the kind that opens the setup.
pub(crate) fn held(pressed: CaptureInstant, released: CaptureInstant) -> bool {
    released
        .checked_duration_since(pressed)
        .is_some_and(|held| held >= crate::dictation::MINIMUM_HOLD_DURATION)
}

/// How a press of the Action shortcut starts, as [`Asking::start`] settles
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Start {
    /// The assistant isn't set up: record nothing; a hold opens the setup
    /// as it comes up.
    Setup,
    /// Record a question: a new Background task after a tap, else Here.
    Record { background: bool },
}

/// One listener's question in progress: what its press settled, and a press
/// that went down before the assistant was set up.
#[derive(Default)]
pub(crate) struct Asking {
    question: Option<Question>,
    /// When a press went down before the assistant was set up: it records
    /// nothing, and held, opens the setup as it comes up.
    setup_press: Option<CaptureInstant>,
}

impl Asking {
    /// A press of the Action shortcut at `at`; `after_tap` when a tap came
    /// just before. Nothing is read yet: a chord or a short tap is
    /// discarded before the hold is real ([`Asking::intend`]).
    pub(crate) fn start(&mut self, after_tap: bool, at: CaptureInstant) -> Start {
        if !crate::app_settings::applied_assistant_ready() {
            self.question = None;
            self.setup_press = Some(at);
            return Start::Setup;
        }
        self.setup_press = None;
        let include_screenshot = crate::app_settings::applied_assistant().include_screenshot;
        self.question = Some(Question::press(after_tap, move || {
            crate::assistant::computer::context_async(include_screenshot)
        }));
        Start::Record {
            background: after_tap,
        }
    }

    /// Whether the question under way is a new Background task (`Some(true)`)
    /// or asks Here, for the HUD; `None` while none is.
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn background(&self) -> Option<bool> {
        self.question
            .as_ref()
            .map(|question| question.lane() == Lane::Background)
    }

    /// Whether the press under way went down before setup and records
    /// nothing.
    pub(crate) fn setting_up(&self) -> bool {
        self.setup_press.is_some()
    }

    /// The press became real (held past the hold threshold with no other
    /// key): a Here question starts reading the desktop now, and its lane
    /// gets ready to answer.
    pub(crate) fn intend(&mut self) {
        if let Some(question) = self.question.as_mut()
            && question.intend()
        {
            crate::assistant::service::prewarm(question.lane());
        }
    }

    /// The release at `at`: the reply its capture's transcript answers,
    /// which then asks the assistant on the question's own thread. `None`
    /// for a press made before setup, which a hold answers by opening the
    /// setup instead.
    pub(crate) fn finish(&mut self, at: CaptureInstant) -> Option<PrintReply> {
        if let Some(pressed) = self.setup_press.take() {
            if held(pressed, at) {
                open_setup();
            }
            return None;
        }
        let question = self
            .question
            .take()
            .unwrap_or_else(|| Question::press(false, nothing_read));
        let follow_up = crate::app_settings::applied_assistant().follow_up_seconds;
        Some(question.ask(move || crate::assistant::focus(follow_up), submit))
    }

    /// The second tap of a double tap came up: its capture ends unheard and
    /// the task list shows or hides, with its tone. Before setup it does
    /// nothing.
    pub(crate) fn double_tap(&mut self) {
        self.question = None;
        if self.setup_press.take().is_some() {
            return;
        }
        toggle_tasks();
        feedback::play(Tone::OnboardingSuccess);
    }

    /// The press ended any other way (a chord, a cancel, a lost key): it
    /// asks nothing.
    pub(crate) fn drop_question(&mut self) {
        self.question = None;
        self.setup_press = None;
    }
}

/// One of the Action shortcut's events, as a Windows or Linux listener
/// hands it to [`Asking::take`].
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    /// The Action shortcut went down, after a tap or not.
    Ask { after_tap: bool },
    /// It came up.
    Finish,
    /// The listener lost its keys, or `hex dictation cancel`.
    Cancel,
    /// The second of two taps came up.
    DoubleTap,
    /// The Stop all tasks shortcut.
    StopAll,
    /// Escape while the assistant held it.
    StopHere,
}

/// What a listener does with its capture after [`Asking::take`].
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub(crate) enum Step {
    /// Nothing: the assistant took it all, or the press records nothing.
    Done,
    /// Start the capture, as the Action shortcut's.
    Start,
    /// Finish the capture; its transcript answers this reply.
    Finish(PrintReply),
    /// End the capture as a cancel does.
    Cancel,
    /// End the capture unheard, as a short tap's (the double tap, whose
    /// task list already showed or hid).
    Discard,
}

#[cfg(not(target_os = "macos"))]
impl Asking {
    /// Takes one of the Action shortcut's events at `at`, and says what the
    /// listener still does with the capture.
    pub(crate) fn take(&mut self, event: Event, at: CaptureInstant) -> Step {
        match event {
            Event::StopAll => {
                stop_all();
                Step::Done
            }
            Event::StopHere => {
                stop_here();
                Step::Done
            }
            Event::Ask { after_tap } => match self.start(after_tap, at) {
                Start::Setup => Step::Done,
                Start::Record { .. } => Step::Start,
            },
            // Nothing recorded; only a hold opens the setup.
            Event::Finish if self.setting_up() => {
                self.finish(at);
                Step::Done
            }
            Event::Cancel | Event::DoubleTap if self.setting_up() => {
                self.drop_question();
                Step::Done
            }
            Event::Finish => self.finish(at).map_or(Step::Done, Step::Finish),
            Event::Cancel => {
                self.drop_question();
                Step::Cancel
            }
            Event::DoubleTap => {
                self.double_tap();
                Step::Discard
            }
        }
    }
}

/// A read that reads nothing, for a release whose press was never seen.
fn nothing_read() -> crate::assistant::computer::ContextHandle {
    crate::assistant::computer::ContextHandle::start(|_| {})
}

/// Hands a request to the assistant. Words for the Here lane end the answer
/// still being read aloud; the press never does, so a tap or silence leaves
/// it.
pub(crate) fn submit(request: Request) {
    if request.origin.lane() == Lane::Here {
        crate::assistant::speech::stop();
    }
    crate::assistant::submit(request);
}

/// The Here tasks among `tasks`: what Escape stops.
pub(crate) fn here_tasks(tasks: &[TaskStatus]) -> Vec<ConversationId> {
    tasks
        .iter()
        .filter(|task| task.origin.lane() == Lane::Here)
        .map(|task| task.conversation.clone())
        .collect()
}

/// Whether a Here task (not a Background or Chats task) is running or
/// waiting.
pub(crate) fn quick_question_running() -> bool {
    crate::assistant::busy() && !here_tasks(&crate::assistant::tasks()).is_empty()
}

/// Whether Escape is the assistant's now: a Here task runs or an answer is
/// read aloud. Otherwise it stays with the app in front.
pub(crate) fn claims_escape() -> bool {
    quick_question_running() || crate::assistant::speech::speaking()
}

/// What Escape does while the assistant holds it: stops the Here tasks and
/// the speech, never a Background task (each has its own stop).
pub(crate) fn stop_here() {
    for conversation in here_tasks(&crate::assistant::tasks()) {
        crate::assistant::cancel(&conversation);
    }
    crate::assistant::speech::stop();
}

/// The Stop all tasks shortcut: every task, and the speech.
pub(crate) fn stop_all() {
    crate::assistant::service::stop_all();
    crate::assistant::speech::stop();
    feedback::play(Tone::Cancel);
}

/// [`claims_escape`] as a listener last checked it, for a keyboard hook or
/// grab that must answer at once (Windows, X11). macOS asks at each tap.
#[cfg(not(target_os = "macos"))]
static ESCAPE_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Checks again whether the assistant holds Escape, `on` being whether the
/// Action shortcut is bound with the assistant on. The listener calls it
/// on each turn of its loop.
#[cfg(not(target_os = "macos"))]
pub(crate) fn refresh_escape(on: bool) {
    ESCAPE_CLAIMED.store(on && claims_escape(), Ordering::Release);
}

/// Whether the assistant held Escape at the listener's last turn.
#[cfg(not(target_os = "macos"))]
pub(crate) fn escape_claimed() -> bool {
    ESCAPE_CLAIMED.load(Ordering::Acquire)
}

/// The Stop all tasks shortcut's key while it is down, so the key's repeats
/// stop nothing twice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StopAllKey<K> {
    held: Option<K>,
}

impl<K: Copy + Eq> StopAllKey<K> {
    /// Whether `key` going `down` is a press of the shortcut, `matches`
    /// saying whether it is the shortcut's key with its modifiers. A repeat
    /// while it is down is no press, and its release lets the next one
    /// count.
    pub(crate) fn press(&mut self, key: K, down: bool, matches: bool) -> bool {
        if !down {
            if self.held == Some(key) {
                self.held = None;
            }
            return false;
        }
        let pressed = self.held != Some(key) && matches;
        if pressed {
            self.held = Some(key);
        }
        pressed
    }

    /// Whether the shortcut's key is down now.
    #[cfg(any(test, not(target_os = "macos")))]
    pub(crate) fn is_down(&self) -> bool {
        self.held.is_some()
    }
}
