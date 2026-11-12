//! Labs › Assistant's Action shortcut, from the listener's side (macOS).
//!
//! Its machine (`Shortcut::Action`) records exactly as dictation does: the
//! same microphone, HUD, sounds, chord discards, and short-tap discards. A
//! press settles what it will ask: in Agent mode a new task; otherwise a quick
//! question with the desktop read on its own thread once the hold is real
//! (past the hold threshold with no other key), so a ⌘C chord reads
//! nothing; the screenshot leaves out Hex's own windows, the HUD included.
//! The release hands the clip to
//! transcription as `TranscriptionTarget::Assistant`, processed like a paste
//! but never pasted, copied, kept for Paste last dictation, or saved to
//! History; its own thread then submits the words. Silence, a discard, or a
//! failed transcription asks nothing. Two complete taps
//! (`HotkeyAction::DoubleTap`) turn Agent mode on or off. Escape stops a
//! running quick question and its speech, and Hex takes Escape from the app in
//! front only while one runs or speaks.

use std::time::Duration;

use color_eyre::Result;

use crate::assistant::computer::ContextHandle;
use crate::assistant::{Context, ConversationId, Origin, Request};
use crate::audio::CaptureInstant;
use crate::control::{ControlReply, PrintReply, PrintedTranscript};
use crate::desktop::events::EventLog;
use crate::dictation::POST_RELEASE_TAIL;
use crate::dictation::audio::DictationAudio;
use crate::dictation::context::ContextSnapshot;
use crate::feedback::{self, Tone};
use crate::parakeet::DictationWorker;
use crate::suppression::{HotkeyAction, InputEvent};

use super::context_watch::ContextWatch;
use super::{DictationSink, FinishResult};

/// How long a question waits for its transcript; a job always answers or
/// drops its reply long before this.
const TRANSCRIPT_WAIT: Duration = Duration::from_secs(300);

/// How long a quick question waits for the desktop read its hold started,
/// which has usually finished while the person spoke. Past it, it asks with
/// what has arrived (the Accessibility part) and drops the rest.
const CONTEXT_WAIT: Duration = Duration::from_secs(3);

/// Starts a desktop read on its own thread (`computer::context_async`).
type ReadDesktop = Box<dyn FnOnce() -> ContextHandle + Send>;

/// What one press of the Action shortcut asks, settled as it goes down.
pub(super) struct Question {
    agent: bool,
    /// How to read the desktop, until the hold is real (quick questions).
    read: Option<ReadDesktop>,
    /// The read under way; dropping it cancels it and frees its screenshot.
    context: Option<ContextHandle>,
}

impl Question {
    /// Settles a press: a task in Agent mode, else a quick question that
    /// reads the desktop with `read_desktop` once [`Question::intend`] says
    /// the hold is real. Nothing is read at the press itself.
    pub(super) fn press(
        agent: bool,
        read_desktop: impl FnOnce() -> ContextHandle + Send + 'static,
    ) -> Self {
        let read = (!agent).then(|| Box::new(read_desktop) as ReadDesktop);
        Self {
            agent,
            read,
            context: None,
        }
    }

    /// The hold passed the hold threshold without another key: start
    /// reading the desktop (once; never in Agent mode).
    pub(super) fn intend(&mut self) {
        if let Some(read) = self.read.take() {
            self.context = Some(read());
        }
    }

    /// The request `text` makes, or none when nothing was said. A quick
    /// question continues the conversation `focus` names; an Agent mode task
    /// always starts its own and looks at the desktop for itself.
    pub(super) fn request(
        self,
        text: &str,
        focus: impl FnOnce() -> Option<ConversationId>,
    ) -> Option<Request> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some(if self.agent {
            Request {
                conversation: None,
                text: text.into(),
                origin: Origin::Background,
                context: Context::default(),
            }
        } else {
            // A hold the loop never saw become real reads now.
            let mut question = self;
            question.intend();
            let context = question
                .context
                .map(|context| context.wait(CONTEXT_WAIT))
                .unwrap_or_default();
            Request {
                conversation: focus(),
                text: text.into(),
                origin: Origin::ActionShortcut,
                context,
            }
        })
    }

    /// The reply the question's transcription answers. Once it does, the
    /// question's own thread builds the request and hands it to `submit`;
    /// a dropped reply (a discard, a cancel, a failure) or no words asks
    /// nothing.
    pub(super) fn ask(
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

/// The listener's Action shortcut: the question its press settled, and
/// whether the tap keeps Escape for the assistant now.
#[derive(Default)]
pub(super) struct Asker {
    question: Option<Question>,
    escape: bool,
}

impl Asker {
    /// Carries out one of the Action machine's actions. Returns whether a
    /// start was accepted, as `handle_hotkey_action` does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn act(
        &mut self,
        action: HotkeyAction,
        at: CaptureInstant,
        input: &DictationAudio,
        worker: &DictationWorker,
        context: &mut ContextWatch,
        device: &str,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<bool> {
        let handle = |action, context: &mut ContextWatch, events: &mut EventLog| {
            super::handle_hotkey_action(
                action,
                false,
                None,
                at,
                input,
                worker,
                context,
                device,
                events,
                indicator,
                release_while_idle,
            )
        };
        match action {
            HotkeyAction::Start => {
                // Reads nothing yet: a chord (⌘C) or a short tap is
                // discarded before the hold is real (`intend`).
                let include_screenshot =
                    crate::app_settings::applied_assistant().include_screenshot;
                let agent = crate::assistant::agent_mode();
                self.question = Some(Question::press(agent, move || {
                    crate::assistant::computer::context_async(include_screenshot)
                }));
                let accepted = handle(action, context, events)?;
                if accepted {
                    // The HUD wears the assistant's colours (and in Agent
                    // mode says "New task"), so a question never looks
                    // like a dictation.
                    indicator.send(
                        crate::dictation::indicator::DictationIndicatorEvent::Asking { agent },
                    );
                } else {
                    self.question = None;
                }
                Ok(accepted)
            }
            HotkeyAction::Finish => {
                let page = context.finish(events)?;
                let question = self.question.take().unwrap_or(Question {
                    agent: crate::assistant::agent_mode(),
                    read: None,
                    context: None,
                });
                let follow_up = crate::app_settings::applied_assistant().follow_up_seconds;
                let reply = question.ask(
                    move || crate::assistant::focus(follow_up),
                    |request| {
                        crate::assistant::submit(request);
                    },
                );
                finish_question(
                    at,
                    input,
                    worker,
                    &page,
                    reply,
                    device,
                    events,
                    indicator,
                    release_while_idle,
                )?;
                Ok(true)
            }
            // The second tap's capture ends unheard, as a short tap's does.
            HotkeyAction::DoubleTap => {
                self.question = None;
                handle(action, context, events)?;
                let on = !crate::assistant::agent_mode();
                crate::assistant::set_agent_mode(on);
                feedback::play(if on {
                    Tone::OnboardingSuccess
                } else {
                    Tone::Cancel
                });
                Ok(true)
            }
            _ => {
                self.question = None;
                handle(action, context, events)
            }
        }
    }

    /// The press became real (held past the hold threshold with no other
    /// key): a quick question starts reading the desktop now.
    pub(super) fn intend(&mut self) {
        if let Some(question) = self.question.as_mut() {
            question.intend();
        }
    }

    /// Whether the tap keeps Escape from the app in front for the assistant
    /// now, remembered for the Escape that arrives next.
    pub(super) fn arm_escape(&mut self, bound: bool, free: bool) -> bool {
        self.escape = super::guards::assistant_claims_escape(bound, free, || {
            quick_question_running() || crate::assistant::speech::speaking()
        });
        self.escape
    }

    /// Escape while the assistant took it stops every quick question and
    /// the speech; Agent mode's tasks keep running (each has its own stop).
    pub(super) fn escape(&self, event: InputEvent) {
        if !self.escape || !event.is_escape_down() {
            return;
        }
        for task in crate::assistant::tasks() {
            if task.origin == Origin::ActionShortcut {
                crate::assistant::cancel(&task.conversation);
            }
        }
        crate::assistant::speech::stop();
    }
}

/// Whether a quick question (not an Agent mode or Chats task) is running or
/// waiting.
fn quick_question_running() -> bool {
    crate::assistant::busy()
        && crate::assistant::tasks()
            .iter()
            .any(|task| task.origin == Origin::ActionShortcut)
}

/// A `hex dictation stop` or `toggle` leaves a question being asked alone:
/// only the Action shortcut's release asks it.
pub(super) fn busy_reply() -> ControlReply {
    ControlReply::error(
        "busy",
        "the Action shortcut is asking the assistant; release it, or cancel with \
         `hex dictation cancel`",
    )
}

/// Ends the question's capture at `at` and hands its clip to transcription
/// for `reply`, as a shortcut dictation's finish does for a paste.
#[allow(clippy::too_many_arguments)]
fn finish_question(
    at: CaptureInstant,
    input: &DictationAudio,
    worker: &DictationWorker,
    page: &ContextSnapshot,
    reply: PrintReply,
    device: &str,
    events: &mut EventLog,
    indicator: DictationSink<'_>,
    release_while_idle: bool,
) -> Result<()> {
    if input.become_intentional(at)? && release_while_idle {
        feedback::play(Tone::DictationStart);
    }
    // A discarded or silent clip drops the reply unanswered: nothing is asked.
    let finished = super::finish_dictation_with(
        input,
        at,
        POST_RELEASE_TAIL,
        |clip, _| worker.transcribe_for_assistant(clip, reply, page.clone()),
        device,
        events,
        indicator,
    )?;
    if matches!(finished, FinishResult::Discarded) {
        feedback::retract_start();
    }
    super::emit_engine_state(events, false, worker, device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn read(text: &str) -> ContextHandle {
        let context = Context {
            frontmost_app: Some(text.into()),
            ..Context::default()
        };
        ContextHandle::start(move |reading| reading.quick(context))
    }

    fn nothing() -> ContextHandle {
        ContextHandle::start(|_| {})
    }

    /// A quick question continues the focused conversation with the desktop
    /// read at its press; an Agent mode task starts its own, without one.
    #[test]
    fn a_hold_asks_with_its_mode_settled_at_the_press() {
        let quick = Question::press(false, || read("Mail"))
            .request("  What's this email about? ", || Some("c1".into()))
            .expect("a quick question");
        assert_eq!(quick.origin, Origin::ActionShortcut);
        assert_eq!(quick.conversation.as_deref(), Some("c1"));
        assert_eq!(quick.text, "What's this email about?");
        assert_eq!(quick.context.frontmost_app.as_deref(), Some("Mail"));

        let task = Question::press(true, || unreachable!("Agent mode reads nothing"))
            .request("File these receipts", || {
                unreachable!("Agent mode continues none")
            })
            .expect("a task");
        assert_eq!(task.origin, Origin::Background);
        assert_eq!(task.conversation, None);
        assert_eq!(task.context.frontmost_app, None);
    }

    /// A press reads nothing until the hold is real, and a press discarded
    /// before then (a ⌘C chord) never reads at all.
    #[test]
    fn the_desktop_is_read_only_once_the_hold_is_real() {
        let (started, reads) = std::sync::mpsc::channel();
        let counted = move |sender: std::sync::mpsc::Sender<()>| {
            move || {
                let _ = sender.send(());
                read("Mail")
            }
        };
        drop(Question::press(false, counted(started.clone())));
        assert!(reads.try_recv().is_err(), "a chord read the desktop");

        let mut question = Question::press(false, counted(started));
        assert!(reads.try_recv().is_err(), "the press read the desktop");
        question.intend();
        question.intend();
        assert_eq!(reads.try_iter().count(), 1);
        let asked = question
            .request("What is this?", || None)
            .expect("a question");
        assert_eq!(asked.context.frontmost_app.as_deref(), Some("Mail"));
    }

    /// Silence asks nothing.
    #[test]
    fn no_words_ask_nothing() {
        assert!(
            Question::press(false, nothing)
                .request("  \n", || None)
                .is_none()
        );
    }

    /// The transcript reaches `submit` from the question's own thread; a
    /// dropped reply (a short tap, a discard, a cancel) and an empty
    /// transcript submit nothing.
    #[test]
    fn the_transcript_submits_and_a_dropped_reply_asks_nothing() {
        let (submitted, asked) = mpsc::channel();
        let submit = move |sender: mpsc::Sender<Request>| {
            move |request| {
                let _ = sender.send(request);
            }
        };
        let reply = Question::press(false, nothing).ask(|| None, submit(submitted.clone()));
        reply.send(PrintedTranscript::Text("Summarize this page".into()));
        let request = asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the question is asked");
        assert_eq!(request.origin, Origin::ActionShortcut);
        assert_eq!(request.text, "Summarize this page");

        drop(Question::press(true, nothing).ask(|| None, submit(submitted.clone())));
        Question::press(false, nothing)
            .ask(|| None, submit(submitted.clone()))
            .send(PrintedTranscript::NoSpeech);
        Question::press(true, nothing)
            .ask(|| None, submit(submitted))
            .send(PrintedTranscript::Text("   ".into()));
        assert!(asked.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
