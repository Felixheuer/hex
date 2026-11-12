//! The assistant's conversation mode and barge-in, carried out with the
//! listener's capture (macOS). [`super::conversation`] decides; this opens,
//! hears, finishes, and closes.
//!
//! - A Here answer (or a question a Here task asks) arrives through
//!   `assistant::subscribe`. Once it has been read aloud, or shown when
//!   answers aren't read, a capture opens without a tone and the HUD gets
//!   `DictationIndicatorEvent::Listening` for `conversation_seconds`.
//! - With `barge_in` on, the capture opens as soon as Hex starts reading,
//!   unshown; a voice over it stops the reading at once
//!   (`speech::stop()`).
//! - Speech makes the capture a question (`Started`, `Asking`, the desktop
//!   read); 900 ms without speech finishes it as a follow-up in the same
//!   conversation, through the Action shortcut's own finish.
//! - Any key (before the shortcut it may start), a dictation, a command's
//!   capture, a pause, a meeting recording, or the setup unfinished closes
//!   it quietly. The capture is the listener's ordinary one, so the
//!   microphone policy (release while idle) holds: the microphone opens for
//!   the window and is released after.

use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use color_eyre::Result;

use crate::assistant::{ConversationId, Event, Origin};
use crate::audio::CaptureInstant;
use crate::desktop::events::EventLog;
use crate::dictation::audio::DictationAudio;
use crate::dictation::context::ContextSnapshot;
use crate::dictation::indicator::DictationIndicatorEvent;
use crate::parakeet::DictationWorker;
use crate::suppression::InputEvent;

use super::DictationSink;
use super::ask::{Question, finish_question, submit};
use super::conversation::{Conversation, Options, Output, Step, db};

/// The listener's conversation mode.
pub(super) struct Talk {
    machine: Conversation,
    /// The capture it opened, by its generation.
    capture: Option<u64>,
    /// The HUD shows it (listening, or a question being heard).
    shown: bool,
    /// The follow-up's desktop read, started when speech begins.
    question: Option<Question>,
    output: Output,
    /// Steps the audio decided, carried out at the loop's next turn.
    pending: Vec<Step>,
}

impl Default for Talk {
    fn default() -> Self {
        Self {
            machine: Conversation::default(),
            capture: None,
            shown: false,
            question: None,
            output: Output::Speakers,
            pending: Vec::new(),
        }
    }
}

impl Talk {
    pub(super) fn conversing(&self) -> bool {
        self.capture.is_some()
    }

    pub(super) fn hear(&mut self, samples: &[f32], rate: u32, indicator: DictationSink<'_>) {
        if self.capture.is_none() {
            return;
        }
        if self.machine.is_hearing() {
            indicator.meter(samples);
        }
        let playback = crate::assistant::speech::level().map(db);
        let steps = self.machine.hear(samples, rate, self.output, playback);
        self.pending.extend(steps);
    }

    /// Any key or modifier ends conversation mode at once, before a shortcut
    /// it may be can start its own capture.
    pub(super) fn key(
        &mut self,
        event: InputEvent,
        input: &DictationAudio,
        indicator: DictationSink<'_>,
    ) {
        if !ends_conversation(event) {
            return;
        }
        self.pending.clear();
        if self.machine.key() == Some(Step::Close) {
            self.close(input, indicator);
        }
    }

    pub(super) fn converse(
        &mut self,
        blocked: bool,
        input: &DictationAudio,
        worker: &DictationWorker,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<()> {
        // Its capture was taken over (a command's start) or lost.
        if self.capture.is_some_and(|generation| {
            generation != input.capture_generation() || !input.is_recording()
        }) {
            self.machine.lost();
            self.reset();
        }
        let ready = crate::app_settings::applied_assistant_ready();
        let settings = crate::app_settings::applied_assistant();
        let now = Instant::now();
        if ready {
            for conversation in answered() {
                self.machine.answered(conversation, now);
            }
        }
        let blocked = blocked || !ready || crate::meeting::watcher::recording();
        let options = Options {
            listen_for: Duration::from_secs(settings.conversation_seconds.into()),
            barge_in: settings.barge_in,
            spoken: settings.speak_answers,
        };
        let mut steps = std::mem::take(&mut self.pending);
        steps.extend(self.machine.tick(
            now,
            crate::assistant::speech::speaking(),
            blocked,
            options,
        ));
        for step in steps {
            self.run(step, input, worker, events, indicator, release_while_idle)?;
        }
        Ok(())
    }

    fn run(
        &mut self,
        step: Step,
        input: &DictationAudio,
        worker: &DictationWorker,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<()> {
        match step {
            Step::Watch => {
                self.open(input)?;
            }
            Step::Listen => {
                if self.capture.is_none() {
                    self.open(input)?;
                }
                if self.capture.is_some() {
                    self.shown = true;
                    indicator.send(DictationIndicatorEvent::Listening);
                }
            }
            Step::Heard | Step::BargeIn => {
                if step == Step::BargeIn {
                    crate::assistant::speech::stop();
                }
                if self.capture.is_none() {
                    return Ok(());
                }
                input.become_intentional(CaptureInstant::now())?;
                if let Some(question) = self.question.as_mut()
                    && question.intend()
                {
                    crate::assistant::service::prewarm(question.lane());
                }
                self.shown = true;
                indicator.send(DictationIndicatorEvent::Started);
                indicator.send(DictationIndicatorEvent::Asking { agent: false });
            }
            Step::Finish(conversation) => {
                if self.capture.is_none() {
                    return Ok(());
                }
                let question = self.question.take().unwrap_or_else(|| {
                    Question::press(false, || {
                        crate::assistant::computer::ContextHandle::start(|_| {})
                    })
                });
                let reply = question.ask(move || Some(conversation), submit);
                self.reset();
                finish_question(
                    CaptureInstant::now(),
                    input,
                    worker,
                    &ContextSnapshot::default(),
                    reply,
                    &input.device_name(),
                    events,
                    indicator,
                    release_while_idle,
                )?;
            }
            Step::Close => self.close(input, indicator),
        }
        Ok(())
    }

    /// Opens a capture of its own, quietly.
    fn open(&mut self, input: &DictationAudio) -> Result<()> {
        if !input.start(CaptureInstant::now())? {
            self.machine.lost();
            return Ok(());
        }
        self.capture = Some(input.capture_generation());
        self.output = match crate::audio::output_route::default_output_route() {
            crate::audio::output_route::OutputRoute::Headphones => Output::Headphones,
            _ => Output::Speakers,
        };
        let include_screenshot = crate::app_settings::applied_assistant().include_screenshot;
        self.question = Some(Question::press(false, move || {
            crate::assistant::computer::context_async(include_screenshot)
        }));
        Ok(())
    }

    /// Closes its capture, if it is still its own, and hides what it showed.
    fn close(&mut self, input: &DictationAudio, indicator: DictationSink<'_>) {
        if self.capture == Some(input.capture_generation())
            && let Err(error) = input.cancel()
        {
            tracing::warn!(%error, "could not close conversation mode's capture");
        }
        if self.shown {
            indicator.send(DictationIndicatorEvent::Discarded);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.capture = None;
        self.shown = false;
        self.question = None;
        self.pending.clear();
    }
}

/// Whether `event` ends conversation mode: a key going down or a modifier
/// changing, Escape and the Action shortcut included.
fn ends_conversation(event: InputEvent) -> bool {
    matches!(
        event,
        InputEvent::Key { down: true, .. } | InputEvent::Flags(_)
    )
}

/// Here answers and questions not yet listened after, from
/// `assistant::subscribe` (subscribed on first use, once the assistant is
/// ready).
fn answered() -> Vec<ConversationId> {
    static SUBSCRIBE: Once = Once::new();
    SUBSCRIBE.call_once(|| {
        crate::assistant::subscribe(Box::new(|event| {
            let mut heard = HEARD.lock().unwrap_or_else(|error| error.into_inner());
            let Heard { here, answered } = &mut *heard;
            if let Some(conversation) = note(event, here) {
                answered.push(conversation);
            }
        }));
    });
    std::mem::take(
        &mut HEARD
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .answered,
    )
}

struct Heard {
    /// Here conversations running now.
    here: Vec<ConversationId>,
    answered: Vec<ConversationId>,
}

static HEARD: Mutex<Heard> = Mutex::new(Heard {
    here: Vec::new(),
    answered: Vec::new(),
});

/// Follows the Here conversations in `here` through `event`, and returns
/// the conversation to listen after: a Here answer, or a question a Here
/// task asks. An escalated task is Background work and no longer listened
/// after.
fn note(event: &Event, here: &mut Vec<ConversationId>) -> Option<ConversationId> {
    match event {
        Event::Queued {
            conversation,
            origin: Origin::ActionShortcut,
            ..
        }
        | Event::Started {
            conversation,
            origin: Origin::ActionShortcut,
            ..
        } => {
            if !here.contains(conversation) {
                here.push(conversation.clone());
            }
            None
        }
        Event::Answer {
            conversation,
            origin,
            ..
        } => {
            here.retain(|running| running != conversation);
            (*origin == Origin::ActionShortcut).then(|| conversation.clone())
        }
        Event::Question { conversation, .. } => {
            here.contains(conversation).then(|| conversation.clone())
        }
        Event::Escalated { conversation }
        | Event::Failed { conversation, .. }
        | Event::Cancelled { conversation } => {
            here.retain(|running| running != conversation);
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(conversation: &str, origin: Origin) -> Event {
        Event::Started {
            conversation: conversation.into(),
            origin,
            question: "?".into(),
        }
    }

    fn answer(conversation: &str, origin: Origin) -> Event {
        Event::Answer {
            conversation: conversation.into(),
            origin,
            text: "!".into(),
        }
    }

    /// Hex listens after a Here answer, and after a question a Here task
    /// asks; never after Background work, or a task that escalated to it.
    #[test]
    fn only_here_answers_and_questions_are_listened_after() {
        let mut here = Vec::new();
        assert_eq!(note(&started("h", Origin::ActionShortcut), &mut here), None);
        assert_eq!(note(&started("b", Origin::Background), &mut here), None);
        let question = |conversation: &str| Event::Question {
            conversation: conversation.into(),
            text: "Which one?".into(),
        };
        assert_eq!(note(&question("h"), &mut here), Some("h".into()));
        assert_eq!(note(&question("b"), &mut here), None);
        assert_eq!(
            note(&answer("h", Origin::ActionShortcut), &mut here),
            Some("h".into())
        );
        assert_eq!(note(&answer("b", Origin::Background), &mut here), None);
        assert!(here.is_empty());

        note(&started("e", Origin::ActionShortcut), &mut here);
        note(
            &Event::Escalated {
                conversation: "e".into(),
            },
            &mut here,
        );
        assert_eq!(note(&question("e"), &mut here), None);
    }

    /// Any key going down or modifier changing ends it; a key coming up or a
    /// mouse click doesn't.
    #[test]
    fn keys_end_conversation_mode() {
        let key = |down| InputEvent::Key {
            code: 0,
            down,
            flags: 0,
        };
        assert!(ends_conversation(key(true)));
        assert!(ends_conversation(InputEvent::Key {
            code: 53,
            down: true,
            flags: 0
        }));
        assert!(ends_conversation(InputEvent::Flags(
            crate::app_settings::RIGHT_COMMAND_MASK
        )));
        assert!(!ends_conversation(key(false)));
        assert!(!ends_conversation(InputEvent::MouseDown));
    }
}
