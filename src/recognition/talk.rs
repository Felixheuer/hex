//! The assistant's conversation mode and barge-in, carried out with the
//! listener's capture (macOS). [`super::conversation`] decides; this opens,
//! hears, finishes, and closes.
//!
//! - A Here answer (or a question a Here task asks) arrives through
//!   `assistant::subscribe`. Once it has been read aloud, or shown when
//!   answers aren't read, a capture opens without a tone and the HUD gets
//!   `DictationIndicatorEvent::Listening` for `conversation_seconds`.
//! - With `barge_in` on, the capture opens as soon as Hex starts reading,
//!   unshown; a voice over it pauses the reading at once
//!   (`speech::pause()`). Its transcript decides: the person's words drop
//!   the answer and ask; nothing, or Hex's own voice heard back, resumes
//!   the reading from the sentence it cut, watched again
//!   (`assistant::shortcut::barge_in`).
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
use super::other_audio::OtherAudio;

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
    /// Whether another app plays audio, which closes the hands-free window.
    other: OtherAudio,
    /// Tells the person's voice from the echo of Hex's on the speakers.
    echo: Option<super::echo::EchoGate>,
    /// A barge-in paused the reading and its capture is being heard.
    barging: bool,
    /// The conversation whose paused reading waits for its barge-in's
    /// transcript; watched again if it resumes.
    barged: Option<ConversationId>,
    /// `speech::resumed_count` when the barge-in finished.
    resumed_seen: u64,
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
            other: OtherAudio::default(),
            echo: None,
            barging: false,
            barged: None,
            resumed_seen: 0,
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
        if self.echo.as_ref().is_none_or(|gate| gate.rate() != rate) {
            self.echo = Some(super::echo::EchoGate::new(rate));
        }
        let gate = self.echo.as_mut();
        let arrived = crate::audio::instant::CaptureInstant::now();
        let mut judge = echo_judge(gate, arrived, rate);
        let steps = self
            .machine
            .hear(samples, rate, self.output, playback, &mut judge);
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
        self.machine
            .set_thresholds(crate::assistant::interaction::tuning::thresholds());
        let now = Instant::now();
        if ready {
            for conversation in answered() {
                self.machine.answered(conversation, now);
            }
            // A barge-in that was only Hex's own voice (or nothing): the
            // reading resumed, so its rest is watched again.
            match rearm(
                self.barged.is_some(),
                self.resumed_seen,
                crate::assistant::speech::resumed_count(),
                crate::assistant::speech::paused(),
            ) {
                Rearm::Watch => {
                    if let Some(conversation) = self.barged.take() {
                        self.machine.answered(conversation, now);
                    }
                }
                Rearm::Forget => self.barged = None,
                Rearm::Wait => {}
            }
        }
        let blocked = blocked || !ready || crate::meeting::watcher::recording();
        let options = Options {
            listen_for: Duration::from_secs(settings.conversation_seconds.into()),
            // Interrupting is by key only (a hold, or „Stopp“ said in one):
            // nothing listens under an answer, so Hex never hears itself.
            barge_in: false,
            spoken: settings.speak_answers,
        };
        // Between two sentences the engine can be idle while more of the
        // answer is still to come: the reading goes on until both are done.
        let speaking = crate::assistant::speech::speaking() || crate::assistant::reading_aloud();
        // Music on the speakers would be heard as a follow-up; Core Audio
        // is only asked while conversation mode is under way.
        if self.machine.is_active() {
            let playing = self.other.playing(now, speaking);
            self.machine.other_audio(playing);
        }
        let mut steps = std::mem::take(&mut self.pending);
        steps.extend(self.machine.tick(now, speaking, blocked, options));
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
                // The capture that watched under the reading (barge-in)
                // holds Hex's own voice: the follow-up starts afresh, its
                // pre-roll keeping a first word said as the reading ended.
                if self.capture.is_some() {
                    self.reopen(input)?;
                } else {
                    self.open(input)?;
                }
                if self.capture.is_some() {
                    self.shown = true;
                    crate::assistant::interaction::conversation_listening(
                        crate::assistant::interaction::Listening::FollowUp,
                    );
                    indicator.send(DictationIndicatorEvent::Listening);
                }
            }
            Step::Heard | Step::BargeIn => {
                if step == Step::BargeIn {
                    // Paused, not stopped: on speakers the voice may have
                    // been Hex's own; its transcript decides
                    // (`assistant::shortcut::barge_in`).
                    if crate::assistant::speech::pause() {
                        tracing::info!("barge-in: paused");
                    }
                    // Its outcome tunes how much voice the next one needs.
                    if let Some(fired) = self.machine.take_fired() {
                        crate::assistant::interaction::tuning::fired(fired);
                    }
                    self.barging = true;
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
                crate::assistant::interaction::conversation_listening(
                    crate::assistant::interaction::Listening::Hearing,
                );
                indicator.send(DictationIndicatorEvent::Started);
                indicator.send(DictationIndicatorEvent::Asking { agent: false });
            }
            Step::Finish(conversation) => {
                if self.capture.is_none() {
                    return Ok(());
                }
                if std::mem::take(&mut self.barging) {
                    self.barged = Some(conversation.clone());
                    self.resumed_seen = crate::assistant::speech::resumed_count();
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

    /// Starts its capture afresh: what the old one recorded is dropped.
    fn reopen(&mut self, input: &DictationAudio) -> Result<()> {
        if self.capture == Some(input.capture_generation())
            && let Err(error) = input.cancel()
        {
            tracing::warn!(%error, "could not restart conversation mode's capture");
        }
        self.capture = None;
        self.open(input)
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
        if self.shown {
            crate::assistant::interaction::conversation_listening(
                crate::assistant::interaction::Listening::Off,
            );
        }
        self.capture = None;
        self.shown = false;
        self.question = None;
        self.barging = false;
        self.pending.clear();
    }
}

/// Whether `event` ends conversation mode: a key going down or a modifier
/// changing, Escape and the Action shortcut included.
/// Judges each frame of a block that `arrived` at `rate` against what Hex
/// played (`audio::playback_reference`): `Some(true)` when it holds the
/// person (or Hex played nothing lately), `Some(false)` when only Hex's
/// echo, `None` while the gate is still learning the room.
fn echo_judge(
    mut gate: Option<&mut super::echo::EchoGate>,
    arrived: crate::audio::instant::CaptureInstant,
    rate: u32,
) -> impl FnMut(&[f32], usize) -> Option<bool> + '_ {
    use super::echo::Verdict;
    move |frame: &[f32], later: usize| {
        let gate = gate.as_deref_mut()?;
        if gate.frame_len() != frame.len() || rate == 0 {
            return None;
        }
        let length = Duration::from_secs_f64(frame.len() as f64 / f64::from(rate));
        let later =
            u64::try_from(Duration::from_secs_f64(later as f64 / f64::from(rate)).as_nanos())
                .unwrap_or(0);
        let end = arrived.as_nanos().saturating_sub(later);
        let reference = |delay: Duration| {
            let delay = u64::try_from(delay.as_nanos()).unwrap_or(0);
            crate::audio::playback_reference::recent(
                crate::audio::instant::CaptureInstant::from_nanos(end.saturating_sub(delay)),
                length,
                rate,
            )
        };
        match gate.judge(frame, &reference) {
            Verdict::NoEcho => Some(true),
            Verdict::Unsure => None,
            Verdict::OverEcho(over) => Some(over),
        }
    }
}

/// What to do about a barge-in's paused reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rearm {
    /// It resumed: watch the rest of the answer again.
    Watch,
    /// It ended (taken over, stopped, or dropped): nothing to watch.
    Forget,
    /// Its transcript isn't in yet.
    Wait,
}

fn rearm(barged: bool, seen: u64, resumed: u64, paused: bool) -> Rearm {
    if !barged {
        Rearm::Wait
    } else if resumed != seen {
        Rearm::Watch
    } else if paused {
        Rearm::Wait
    } else {
        Rearm::Forget
    }
}

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

    #[test]
    fn a_resumed_reading_is_watched_again_and_a_taken_over_one_forgotten() {
        use super::{Rearm, rearm};
        assert_eq!(rearm(false, 0, 1, false), Rearm::Wait, "no barge-in");
        assert_eq!(rearm(true, 3, 3, true), Rearm::Wait, "transcript not in");
        assert_eq!(rearm(true, 3, 4, false), Rearm::Watch, "resumed");
        assert_eq!(rearm(true, 3, 3, false), Rearm::Forget, "taken over");
    }
}
