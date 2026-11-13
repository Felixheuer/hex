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

mod barge_in;
mod latin;
pub(crate) mod spoken_stop;
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
// Called by the app (`rune_ui::window_link`) as it starts.
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
    /// The question a job waited on when the key went down: a hold answers
    /// it only if the person could have heard or seen it by then.
    asked_at_press: Option<ConversationId>,
    /// A press of the Action shortcut on Linux or Windows ([`press`]): once
    /// real it quiets Agent-Desk and holds announcements back until it's let go
    /// (macOS's listener does both itself; conversation mode's captures
    /// never do).
    hold: bool,
    /// The hold's listening, while it's real; ends when dropped.
    holding: Option<Holding>,
    /// Agent-Desk was reading aloud as the capture opened (conversation mode's
    /// watch under an answer, a hold over it): its transcript may begin
    /// with Agent-Desk's own voice.
    overheard: bool,
}

/// A hold's listening state (`interaction::hold_listening`): on while this
/// lives.
struct Holding;

impl Drop for Holding {
    fn drop(&mut self) {
        crate::assistant::interaction::hold_listening(false);
    }
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
            asked_at_press: crate::assistant::inbox()
                .active
                .map(|asked| asked.conversation),
            hold: false,
            holding: None,
            overheard: crate::assistant::speech::speaking() || crate::assistant::reading_aloud(),
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
        if self.hold && self.holding.is_none() {
            crate::assistant::interaction::hold_listening(true);
            if crate::assistant::speech::speaking() || crate::assistant::reading_aloud() {
                crate::assistant::interaction::hush();
            }
            self.holding = Some(Holding);
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
        if let Some(rest) = crate::assistant::route::spoken_background(text) {
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
        // A job's question that came up while the person held the key
        // (announcements wait meanwhile) wasn't heard: the hold goes where
        // it would have gone without it.
        let focused = focus();
        let conversation = match (&focused, crate::assistant::inbox().active) {
            (Some(focused), Some(active))
                if *focused == active.conversation
                    && self.asked_at_press.as_ref() != Some(focused) =>
            {
                crate::assistant::service::focus_past_inbox(
                    crate::app_settings::applied_assistant().follow_up_seconds,
                )
            }
            _ => focused,
        };
        Some(Request {
            conversation,
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
        mut self,
        focus: impl FnOnce() -> Option<ConversationId> + Send + 'static,
        submit: impl FnOnce(Request) + Send + 'static,
    ) -> PrintReply {
        // Released: the hold no longer listens.
        drop(self.holding.take());
        let overheard = self.overheard;
        let (reply, transcript) = PrintReply::channel();
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-question".into())
            .spawn(move || {
                // A barge-in paused the reading: what was heard decides
                // whether it resumes or the person takes over.
                let barged = crate::assistant::speech::paused();
                let Ok(PrintedTranscript::Text(text)) = transcript.recv_timeout(TRANSCRIPT_WAIT)
                else {
                    if barged {
                        barge_in::settle(barge_in::AfterBargeIn::Resume("empty"));
                    }
                    return;
                };
                // A capture that listened under the reading (a barge-in,
                // conversation mode's window) heard Agent-Desk's voice before the
                // person's: only what follows it is theirs.
                let text = if overheard || barged {
                    crate::assistant::speech::own_echo::without_own_words(&text)
                } else {
                    text
                };
                if barged {
                    let after = barge_in::after_barge_in(
                        Some(&text),
                        crate::assistant::speech::own_echo::heard_back,
                    );
                    barge_in::settle(after);
                    if matches!(after, barge_in::AfterBargeIn::Resume(_)) {
                        return;
                    }
                }
                // "Stopp" ends the speech (and the Here task), asking
                // nothing; "Stopp alle Aufgaben" stops every task.
                // Agent-Desk's own reading, heard back through the speakers.
                if !text.chars().any(char::is_alphanumeric)
                    || crate::assistant::speech::own_echo::heard_back(&text)
                {
                    tracing::info!("dropped a question that was Agent-Desk's own voice");
                    return;
                }
                // „Brich die Shop-Aufgabe ab“: that one task stops.
                if let Some(names) = spoken_stop::named_stop(&text) {
                    let tasks = crate::assistant::tasks();
                    if let Some(task) = spoken_stop::task_named(&names, &tasks) {
                        tracing::info!(task = task.conversation, "stopped by name");
                        crate::assistant::cancel(&task.conversation);
                        crate::feedback::play(crate::feedback::Tone::Cancel);
                        return;
                    }
                }
                match spoken_stop::spoken_stop(&text) {
                    // A job asks ("abschicken oder abbrechen?") and nothing
                    // else is going on: „Abbrechen“ answers it.
                    Some(spoken_stop::SpokenStop::Quiet) if answers_a_question() => {}
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

/// Whether Agent-Desk takes Escape for the assistant now: the Action shortcut is
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
    // „Sei still“ means it: jobs' summaries and questions wait a minute
    // instead of starting 200 ms later (their bubbles still show).
    crate::assistant::service::quiet_announcements(QUIET_AFTER_STOP);
}

/// How long announcements wait after Escape or a spoken stop.
const QUIET_AFTER_STOP: Duration = Duration::from_secs(60);

/// Whether a spoken stop word answers a job's question rather than
/// stopping: one waits, nothing is read aloud, and no quick question runs.
fn answers_a_question() -> bool {
    crate::assistant::inbox().active.is_some()
        && !crate::assistant::speech::speaking()
        && !crate::assistant::reading_aloud()
        && !quick_question_running()
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

/// The desktop read a hold starts, the one place every OS's listener and
/// conversation mode take it from: the Accessibility part, and a screenshot
/// only when `assistant.include_screenshot` asks for one (off by default:
/// the agent looks with its desktop tools when it needs to).
pub(crate) fn desktop_read() -> ContextHandle {
    let include_screenshot = crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.include_screenshot)
        .unwrap_or_default();
    crate::assistant::computer::context_async(include_screenshot)
}

/// The question a press of the Action shortcut starts on Linux and Windows,
/// with the desktop read the settings ask for.
pub(crate) fn press(after_tap: bool) -> Question {
    let mut question = Question::press(after_tap, desktop_read);
    question.hold = true;
    question
}

/// The reply a released question's transcription answers: its words go to
/// the assistant, continuing the focused conversation.
pub(crate) fn reply(question: Question) -> PrintReply {
    let follow_up = crate::app_settings::assistant_runtime()
        .map(|(_, settings)| settings.follow_up_seconds)
        .unwrap_or_default();
    question.ask(move || crate::assistant::focus(follow_up), submit)
}
