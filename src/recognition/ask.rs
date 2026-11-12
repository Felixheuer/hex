//! Labs › Assistant's Action shortcut, from the listener's side (macOS).
//!
//! Its machine (`Shortcut::Action`) records exactly as dictation does: the
//! same microphone, HUD, sounds, chord discards, and short-tap discards. A
//! press settles what it will ask: in Agent mode a new task; otherwise a quick
//! question with the desktop as it looked at the press, read on its own
//! thread before the HUD changes the screen. The release hands the clip to
//! transcription as `TranscriptionTarget::Assistant`, processed like a paste
//! but never pasted, copied, kept for Paste last dictation, or saved to
//! History; its own thread then submits the words. Silence, a discard, or a
//! failed transcription asks nothing. Two complete taps
//! (`HotkeyAction::DoubleTap`) turn Agent mode on or off. Escape stops a
//! running quick question and its speech, and Hex takes Escape from the app in
//! front only while one runs or speaks.

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use color_eyre::Result;

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

/// How long a quick question waits for the desktop read at its press, which
/// has usually finished while the person spoke. Past it, it asks without.
const CONTEXT_WAIT: Duration = Duration::from_secs(3);

/// What one press of the Action shortcut asks, settled as it goes down.
pub(super) struct Question {
    agent: bool,
    /// The desktop at the press (quick questions only).
    context: Option<Receiver<Context>>,
}

impl Question {
    /// Settles a press: a task in Agent mode, else a quick question that
    /// reads the desktop with `read_desktop` on its own thread, so the key
    /// path never waits for it.
    pub(super) fn press(
        agent: bool,
        read_desktop: impl FnOnce() -> Context + Send + 'static,
    ) -> Self {
        let context = (!agent).then(|| {
            let (sender, receiver) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new()
                .name("hex-assistant-context".into())
                .spawn(move || {
                    let _ = sender.send(read_desktop());
                });
            if let Err(error) = spawned {
                tracing::warn!(%error, "could not read the desktop for a question");
            }
            receiver
        });
        Self { agent, context }
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
                origin: Origin::AgentMode,
                context: Context::default(),
            }
        } else {
            let context = self
                .context
                .and_then(|context| context.recv_timeout(CONTEXT_WAIT).ok())
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
                // Read before the HUD appears; dropped if the start is refused.
                let include_screenshot =
                    crate::app_settings::applied_assistant().include_screenshot;
                let agent = crate::assistant::agent_mode();
                self.question = Some(Question::press(agent, move || {
                    crate::assistant::computer::context(include_screenshot)
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

    fn read(text: &str) -> Context {
        Context {
            frontmost_app: Some(text.into()),
            ..Context::default()
        }
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
        assert_eq!(task.origin, Origin::AgentMode);
        assert_eq!(task.conversation, None);
        assert_eq!(task.context.frontmost_app, None);
    }

    /// Silence asks nothing.
    #[test]
    fn no_words_ask_nothing() {
        assert!(
            Question::press(false, Context::default)
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
        let reply =
            Question::press(false, Context::default).ask(|| None, submit(submitted.clone()));
        reply.send(PrintedTranscript::Text("Summarize this page".into()));
        let request = asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the question is asked");
        assert_eq!(request.origin, Origin::ActionShortcut);
        assert_eq!(request.text, "Summarize this page");

        drop(Question::press(true, Context::default).ask(|| None, submit(submitted.clone())));
        Question::press(false, Context::default)
            .ask(|| None, submit(submitted.clone()))
            .send(PrintedTranscript::NoSpeech);
        Question::press(true, Context::default)
            .ask(|| None, submit(submitted))
            .send(PrintedTranscript::Text("   ".into()));
        assert!(asked.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
