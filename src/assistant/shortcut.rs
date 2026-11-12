//! The Action shortcut's questions, shared by every OS's listener (the
//! macOS one in `recognition::ask`, Linux's in `linux::dictation`, and
//! Windows' in `windows::dictation`): what one press asks, the spoken
//! openings that send it to the Background lane, the gestures' hooks (the
//! task list, the setup), and what Escape and Stop all tasks stop.
//!
//! A press settles its lane ([`Question::press`]); the hold becoming real
//! starts the desktop read ([`Question::intend`]); the release hands the
//! transcription a [`PrintReply`] ([`Question::ask`]) whose words become the
//! [`Request`] on the question's own thread, so nothing is pasted, copied,
//! or kept for History.

mod latin;
mod spoken_stop;
use std::sync::Mutex;
use std::time::Duration;

use crate::assistant::computer::ContextHandle;
use crate::assistant::{Context, ConversationId, Lane, Origin, Request, TaskStatus};
use crate::audio::CaptureInstant;
use crate::control::{PrintReply, PrintedTranscript};

/// How long a question waits for its transcript; a job always answers or
/// drops its reply long before this.
const TRANSCRIPT_WAIT: Duration = Duration::from_secs(300);

/// How long a quick question waits for the desktop read its hold started,
/// which has usually finished while the person spoke. Past it, it asks with
/// what has arrived (the Accessibility part) and drops the rest.
const CONTEXT_WAIT: Duration = Duration::from_secs(3);

/// Spoken openings that send a question to the Background lane, as words
/// compared without case or the punctuation around them. `true`: only with a
/// colon after it ("Hintergrund: …"), since "Hintergrund ändern" asks about
/// the background itself.
const SPOKEN_BACKGROUND: [(&[&str], bool); 5] = [
    (&["im", "hintergrund"], false),
    (&["in", "the", "background"], false),
    (&["nebenbei"], false),
    (&["parallel"], false),
    (&["hintergrund"], true),
];

/// Starts a desktop read on its own thread (`computer::context_async`).
type ReadDesktop = Box<dyn FnOnce() -> ContextHandle + Send>;

/// What a gesture of the Action shortcut calls (set by its owner).
type Hook = Box<dyn Fn() + Send + Sync>;

static TOGGLE_TASKS: Mutex<Option<Hook>> = Mutex::new(None);

/// Sets what a double tap of the Action shortcut does: show or hide the
/// HUD's expanded task list. It is called on the listener's thread, so post
/// to your own and return at once. A later call replaces an earlier one;
/// until the first, a double tap only plays its tone.
// Called by the overlays (`rune_ui::assistant_overlay`) as they install.
#[allow(dead_code)]
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
// Called by the app (`macos_rune`) as it starts.
#[allow(dead_code)]
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

/// `text` without its spoken opening when it asks for the Background lane
/// ("Im Hintergrund, räum den Downloads-Ordner auf" becomes "räum den
/// Downloads-Ordner auf"); `None` when it doesn't start with one. A word
/// that merely starts like one ("Parallelogramm", "Hintergrundbild") is not
/// one.
pub(crate) fn spoken_background(text: &str) -> Option<&str> {
    let text = text.trim();
    SPOKEN_BACKGROUND.iter().find_map(|&(words, colon)| {
        let mut rest = text;
        let mut last = "";
        for &word in words {
            let trimmed = rest.trim_start();
            let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
            let (spoken, after) = trimmed.split_at(end);
            let bare = spoken.trim_matches(|c: char| !c.is_alphanumeric());
            if bare.to_lowercase() != word {
                return None;
            }
            last = spoken;
            rest = after;
        }
        // "Hintergrund:" or "Hintergrund :".
        if colon && !last.ends_with(':') {
            rest = rest.trim_start().strip_prefix(':')?;
        }
        Some(
            rest.trim_start_matches(|c: char| !c.is_alphanumeric())
                .trim_end(),
        )
    })
}

/// What one press of the Action shortcut asks, settled as it goes down.
pub(crate) struct Question {
    /// The press followed a tap: a new Background task.
    background: bool,
    /// How to read the desktop, until the hold is real (Here questions).
    read: Option<ReadDesktop>,
    /// The read under way; dropping it cancels it and frees its screenshot.
    context: Option<ContextHandle>,
    /// The hold became real.
    real: bool,
}

impl Question {
    /// Settles a press: a new Background task after a tap, else a Here
    /// question that reads the desktop with `read_desktop` once
    /// [`Question::intend`] says the hold is real. Nothing is read at the
    /// press itself.
    pub(crate) fn press(
        background: bool,
        read_desktop: impl FnOnce() -> ContextHandle + Send + 'static,
    ) -> Self {
        let read = (!background).then(|| Box::new(read_desktop) as ReadDesktop);
        Self {
            background,
            read,
            context: None,
            real: false,
        }
    }

    /// The lane the press asks in, as settled at the press.
    pub(crate) fn lane(&self) -> Lane {
        if self.background {
            Lane::Background
        } else {
            Lane::Here
        }
    }

    /// The hold passed the hold threshold without another key: start
    /// reading the desktop (once; never for a Background task). Returns
    /// whether the hold only now became real.
    pub(crate) fn intend(&mut self) -> bool {
        if let Some(read) = self.read.take() {
            self.context = Some(read());
        }
        !std::mem::replace(&mut self.real, true)
    }

    /// The request `text` makes, or none when nothing was said. A hold
    /// continues the conversation `focus` names (a task waiting on a
    /// question, a running Here task, or a recent one), and the assistant
    /// decides how to answer; a forced Background task, or words that ask
    /// for one first, start their own and look for themselves.
    pub(crate) fn request(
        mut self,
        text: &str,
        focus: impl FnOnce() -> Option<ConversationId>,
    ) -> Option<Request> {
        let language = crate::app_settings::applied_transcription_language();
        let spelled = latin::latin_strays(text.trim(), language.as_deref());
        let text = spelled.as_ref();
        let background = |text: &str| {
            (!text.is_empty()).then(|| Request {
                conversation: None,
                text: text.into(),
                origin: Origin::Background,
                context: Context::default(),
            })
        };
        if text.is_empty() {
            return None;
        }
        if self.background {
            return background(text);
        }
        if let Some(rest) = spoken_background(text) {
            // Dropping the read cancels it: a Background task sends no
            // screenshot.
            return background(rest);
        }
        // A hold the loop never saw become real reads now.
        self.intend();
        let context = self
            .context
            .map(|context| context.wait(CONTEXT_WAIT))
            .unwrap_or_default();
        Some(Request {
            conversation: focus(),
            text: text.into(),
            origin: Origin::ActionShortcut,
            context,
        })
    }

    /// The reply the question's transcription answers. Once it does, the
    /// question's own thread builds the request and hands it to `submit`;
    /// a dropped reply (a discard, a cancel, a failure) or no words asks
    /// nothing.
    pub(crate) fn ask(
        self,
        focus: impl FnOnce() -> Option<ConversationId> + Send + 'static,
        submit: impl FnOnce(Request) + Send + 'static,
    ) -> PrintReply {
        let (reply, transcript) = PrintReply::channel();
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-question".into())
            .spawn(move || {
                let Ok(PrintedTranscript::Text(text)) = transcript.recv_timeout(TRANSCRIPT_WAIT)
                else {
                    return;
                };
                // "Stopp" ends the speech (and the Here task), asking
                // nothing; "Stopp alle Aufgaben" stops every task.
                // Hex's own reading, heard back through the speakers.
                if crate::assistant::speech::own_echo::heard_back(&text) {
                    tracing::info!("dropped a question that was Hex's own voice");
                    return;
                }
                match spoken_stop::spoken_stop(&text) {
                    Some(spoken_stop::SpokenStop::Quiet) => return escape(),
                    Some(spoken_stop::SpokenStop::All) => return stop_all(),
                    None => {}
                }
                if let Some(request) = self.request(&text, focus) {
                    submit(request);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not wait for a question's transcript");
        }
        reply
    }
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

/// Whether Hex takes Escape for the assistant now: the Action shortcut is
/// bound, nothing records or holds Escape otherwise (`free`), and a Here
/// task runs or an answer is read aloud.
pub(crate) fn claims_escape(bound: bool, free: bool) -> bool {
    bound && free && (quick_question_running() || crate::assistant::speech::speaking())
}

/// Escape while the assistant took it: stops the Here tasks and the speech,
/// never a Background task (each has its own stop).
pub(crate) fn escape() {
    for conversation in here_tasks(&crate::assistant::tasks()) {
        crate::assistant::cancel(&conversation);
    }
    crate::assistant::interaction::hush();
}

/// The Stop all tasks shortcut: every task and the speech stop.
pub(crate) fn stop_all() {
    crate::assistant::service::stop_all();
    crate::assistant::speech::stop();
    crate::feedback::play(crate::feedback::Tone::Cancel);
}

/// A double tap: the task list shows or hides, with the success tone.
pub(crate) fn double_tap() {
    toggle_tasks();
    crate::feedback::play(crate::feedback::Tone::OnboardingSuccess);
}

/// A press of the Action shortcut while the assistant isn't set up: held
/// past the hold threshold for `held_for`, it opens the setup.
pub(crate) fn unready_release(held_for: Duration) {
    if held_for >= crate::dictation::MINIMUM_HOLD_DURATION {
        open_setup();
    }
}

/// The question a press of the Action shortcut starts on Linux and Windows,
/// with the desktop read the settings ask for.
pub(crate) fn press(after_tap: bool) -> Question {
    let include_screenshot = crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.include_screenshot)
        .unwrap_or(true);
    Question::press(after_tap, move || {
        crate::assistant::computer::context_async(include_screenshot)
    })
}

/// The reply a released question's transcription answers: its words go to
/// the assistant, continuing the focused conversation.
pub(crate) fn reply(question: Question) -> PrintReply {
    let follow_up = crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.follow_up_seconds)
        .unwrap_or_default();
    question.ask(move || crate::assistant::focus(follow_up), submit)
}
