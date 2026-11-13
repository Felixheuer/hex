//! The assistant's conversation mode and barge-in (macOS): what the
//! microphone does after a Here answer, as a pure state machine fed by the
//! answer's events, whether Hex is reading aloud, and 20 ms frames of
//! microphone level. `recognition::ask` carries out its [`Step`]s with the
//! listener's capture.
//!
//! ```ts
//! Idle
//! └── answered(c) -> Answer(c)
//!     ├── barge_in, and Hex reads it aloud -> Step::Watch: a capture opens, unshown
//!     │   └── a voice for 240 ms -> Step::BargeIn -> Hearing(c)
//!     │       headphones: any speech; speakers: speech the echo of Hex's
//!     │       own voice doesn't explain (`echo`, from what Hex played), or,
//!     │       until that has learnt the room, speech louder than the answer
//!     │       itself (speech::level() + 6 dB)
//!     └── the reading ends (quiet for 400 ms: a gap between two sentences,
//!         or a sentence still being made, isn't the end), or never starts
//!         within 1.5 s, or answers aren't read aloud
//!         -> Step::Listen -> Listening(c) for conversation_seconds
//!         ├── speech for 160 ms -> Step::Heard -> Hearing(c)
//!         └── nothing in time -> Step::Close -> Idle
//! Hearing(c) -> 900 ms without speech, or 60 s -> Step::Finish(c) -> Idle
//! A key, a dictation, a meeting recording, the setup unfinished -> Step::Close -> Idle
//!
//! Another app playing audio (music, a video, a call; `other_audio`):
//! - Answer: no voice barge-in (a capture open under it closes); holding
//!   the key and saying "Stopp", or Escape, still stops the reading.
//! - the reading ends -> Step::Close or nothing -> Idle: no hands-free window.
//! - Listening -> Step::Close -> Idle, the moment it starts.
//! - Hearing goes on: the person is already speaking.
//! ```
//!
//! Speech is the dictation pipeline's frame test (`SpeechFrames`: 12 dB over
//! an adaptive noise floor, never under -55 dBFS), counted in runs that
//! forgive two quiet frames, so a click or a key never opens a question.

use std::time::{Duration, Instant};

use crate::assistant::ConversationId;
use crate::assistant::interaction::tuning::{Fired, Route, Thresholds};
use crate::transcription::pause_segmenter::SpeechFrames;

/// How long an answer may wait to start being read aloud before Hex counts
/// it as shown only.
const SPEAK_GRACE: Duration = Duration::from_millis(1_500);
/// How long the reading must have stopped before it counts as over. Between
/// two sentences (the next still streaming or being made) `speaking` drops
/// for a moment; listening then would hear Hex's own next sentence.
const READING_SETTLE: Duration = Duration::from_millis(400);
/// Speech frames in a run that start a follow-up: 160 ms, the speech
/// detector's shortest speech.
const ONSET_FRAMES: u32 = 8;
// The speech frames a run needs to interrupt an answer (240 ms on
// headphones; 500 ms on speakers, judged against the echo of Hex's own voice,
// since the echo judge errs on loud built-in speakers and a false
// interruption cuts the answer) and the margin over the answer's playback
// level before the echo is learnt start from guesses and learn from each
// barge-in's outcome (`assistant::interaction::tuning`).
/// Quiet frames a run forgives.
const RUN_GAP_FRAMES: u32 = 2;
/// Quiet frames that end a follow-up: 900 ms.
const END_FRAMES: u32 = 45;
/// The longest follow-up: 60 s.
const MAX_FRAMES: u32 = 3_000;
/// 20 ms frames.
const FRAMES_PER_SECOND: u32 = 50;

/// Where the answer is heard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Output {
    /// Headphones or AirPods: the microphone can't hear Hex's voice.
    Headphones,
    /// Speakers, or anything Hex can't tell: it can.
    Speakers,
}

/// The settings conversation mode reads, as last applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct Options {
    /// `assistant.conversation_seconds`; zero turns listening off.
    pub(super) listen_for: Duration,
    /// `assistant.barge_in`.
    pub(super) barge_in: bool,
    /// `assistant.speak_answers`: answers are read aloud.
    pub(super) spoken: bool,
}

/// What the listener does next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Step {
    /// Open a capture under the answer being read, without showing it, to
    /// hear a barge-in.
    Watch,
    /// Open a capture (unless one is open) and show the soft listening state.
    Listen,
    /// Speech began: the capture becomes a question.
    Heard,
    /// Speech began over the answer: stop reading, then as [`Step::Heard`].
    BargeIn,
    /// The follow-up ended: ask it in this conversation.
    Finish(ConversationId),
    /// Close the capture quietly; nothing is asked.
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Phase {
    Idle,
    /// A Here answer arrived and may still be read aloud.
    Answer {
        conversation: ConversationId,
        since: Instant,
        spoke: bool,
        /// A capture listens under it for a barge-in.
        open: bool,
        /// Since when nothing has been read, after something was.
        quiet_since: Option<Instant>,
    },
    /// Listening hands-free for a follow-up until `until`.
    Listening {
        conversation: ConversationId,
        until: Instant,
    },
    /// Hearing a follow-up.
    Hearing {
        conversation: ConversationId,
        quiet: u32,
        frames: u32,
    },
}

/// One 20 ms frame of microphone audio.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    pub(super) speech: bool,
    pub(super) level_db: f32,
    /// Whether it holds more than the echo of Hex's own voice
    /// (`echo::EchoGate`): `Some(true)` also when Hex played nothing lately;
    /// `None` while it can't tell yet.
    pub(super) over_echo: Option<bool>,
}

/// Cuts microphone audio into 20 ms frames and classifies them.
#[derive(Default)]
struct Ears {
    frame: Vec<f32>,
    speech: SpeechFrames,
}

impl Ears {
    /// `echo(frame, later)` judges a frame against Hex's own voice; `later`
    /// is how many samples of this block arrived after the frame ended.
    fn push(
        &mut self,
        samples: &[f32],
        rate: u32,
        echo: &mut dyn FnMut(&[f32], usize) -> Option<bool>,
        mut each: impl FnMut(Frame),
    ) {
        let size = (rate / FRAMES_PER_SECOND).max(1) as usize;
        let mut samples = samples;
        while !samples.is_empty() {
            let take = samples.len().min(size - self.frame.len());
            self.frame.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.frame.len() == size {
                each(Frame {
                    speech: self.speech.is_speech(&self.frame),
                    level_db: level_db(&self.frame),
                    over_echo: echo(&self.frame, samples.len()),
                });
                self.frame.clear();
            }
        }
    }
}

/// A frame's level in dBFS, with its DC offset removed.
fn level_db(frame: &[f32]) -> f32 {
    let count = frame.len().max(1) as f32;
    let mean = frame.iter().sum::<f32>() / count;
    let energy = frame
        .iter()
        .map(|sample| (sample - mean) * (sample - mean))
        .sum::<f32>()
        / count;
    10.0 * energy.max(1e-18).log10()
}

/// The machine. One per listener.
pub(super) struct Conversation {
    phase: Phase,
    ears: Ears,
    /// Another app plays audio now, which the microphone would hear.
    other_audio: bool,
    /// Speech frames in the current run, and quiet frames since its last.
    run: u32,
    gap: u32,
    /// The voice a barge-in needs, as learnt (`set_thresholds`).
    thresholds: Thresholds,
    /// The last barge-in, until the listener takes it (`take_fired`).
    fired: Option<Fired>,
}

impl Default for Conversation {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ears: Ears::default(),
            other_audio: false,
            run: 0,
            thresholds: Thresholds::DEFAULT,
            fired: None,
            gap: 0,
        }
    }
}

impl Conversation {
    /// Whether a capture of its own is open.
    pub(super) fn is_open(&self) -> bool {
        match self.phase {
            Phase::Idle => false,
            Phase::Answer { open, .. } => open,
            Phase::Listening { .. } | Phase::Hearing { .. } => true,
        }
    }

    /// Whether anything is going on (an answer, listening, hearing): the
    /// only time other apps' audio needs reading.
    pub(super) fn is_active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Whether another app plays audio now (`other_audio::OtherAudio`);
    /// the next [`Self::tick`] and every frame act on it.
    pub(super) fn other_audio(&mut self, playing: bool) {
        self.other_audio = playing;
    }

    /// Whether it hears a follow-up now.
    pub(super) fn is_hearing(&self) -> bool {
        matches!(self.phase, Phase::Hearing { .. })
    }

    /// A Here answer (or a question the task asks) arrived in
    /// `conversation`. A follow-up being heard keeps going.
    pub(super) fn answered(&mut self, conversation: ConversationId, now: Instant) {
        if self.is_hearing() {
            return;
        }
        let open = self.is_open();
        self.phase = Phase::Answer {
            conversation,
            since: now,
            spoke: false,
            open,
            quiet_since: None,
        };
    }

    /// Moves on with time: `speaking` is whether Hex reads aloud now or
    /// still has some of the answer to read (`speech::speaking()` or
    /// `assistant::reading_aloud()`);
    /// `blocked`, whether something else owns the microphone or the
    /// assistant can't answer (a dictation, a meeting recording, setup).
    pub(super) fn tick(
        &mut self,
        now: Instant,
        speaking: bool,
        blocked: bool,
        options: Options,
    ) -> Option<Step> {
        if blocked {
            return self.end();
        }
        match &mut self.phase {
            Phase::Idle | Phase::Hearing { .. } => None,
            Phase::Listening { until, .. } => {
                if now >= *until || self.other_audio {
                    self.end()
                } else {
                    None
                }
            }
            Phase::Answer {
                conversation,
                since,
                spoke,
                open,
                quiet_since,
            } => {
                if speaking {
                    *spoke = true;
                    *quiet_since = None;
                    if !options.barge_in || self.other_audio {
                        // Without barge-in nothing listens under an answer,
                        // not even a capture left open by the last one; nor
                        // while music plays, whose vocals would count.
                        return std::mem::take(open).then_some(Step::Close);
                    }
                    if !*open {
                        *open = true;
                        self.run = 0;
                        self.gap = 0;
                        return Some(Step::Watch);
                    }
                    return None;
                }
                let over = if *spoke {
                    let quiet = *quiet_since.get_or_insert(now);
                    now.duration_since(quiet) >= READING_SETTLE
                } else {
                    !options.spoken || now.duration_since(*since) >= SPEAK_GRACE
                };
                if !over {
                    return None;
                }
                if options.listen_for.is_zero() || self.other_audio {
                    return self.end();
                }
                self.phase = Phase::Listening {
                    conversation: conversation.clone(),
                    until: now + options.listen_for,
                };
                self.run = 0;
                self.gap = 0;
                Some(Step::Listen)
            }
        }
    }

    /// Microphone audio at `rate` from its open capture. `output` is where
    /// the answer plays; `playback_db`, how loud Hex reads it now (dBFS),
    /// if known; `echo`, the judge of each frame against Hex's own voice
    /// (see [`Frame::over_echo`]), asked only while an answer plays.
    pub(super) fn hear(
        &mut self,
        samples: &[f32],
        rate: u32,
        output: Output,
        playback_db: Option<f32>,
        echo: &mut dyn FnMut(&[f32], usize) -> Option<bool>,
    ) -> Vec<Step> {
        let mut steps = Vec::new();
        if !self.is_open() {
            return steps;
        }
        let judging = matches!(self.phase, Phase::Answer { .. }) && output == Output::Speakers;
        let mut none = |_: &[f32], _: usize| None;
        let echo: &mut dyn FnMut(&[f32], usize) -> Option<bool> =
            if judging { echo } else { &mut none };
        let mut frames = Vec::new();
        self.ears
            .push(samples, rate, echo, |frame| frames.push(frame));
        for frame in frames {
            steps.extend(self.frame(frame, output, playback_db));
        }
        steps
    }

    /// One frame of the open capture's audio.
    pub(super) fn frame(
        &mut self,
        frame: Frame,
        output: Output,
        playback_db: Option<f32>,
    ) -> Option<Step> {
        match &mut self.phase {
            Phase::Idle => None,
            Phase::Hearing {
                conversation,
                quiet,
                frames,
            } => {
                *frames += 1;
                *quiet = if frame.speech { 0 } else { *quiet + 1 };
                if *quiet < END_FRAMES && *frames < MAX_FRAMES {
                    return None;
                }
                let conversation = conversation.clone();
                self.phase = Phase::Idle;
                Some(Step::Finish(conversation))
            }
            Phase::Answer {
                conversation, open, ..
            } => {
                if !*open || self.other_audio {
                    return None;
                }
                // Only with headphones: on speakers Hex's own voice comes
                // back too loud and too varied for any judge to tell from
                // the person (it interrupted itself over and over); there a
                // hold or „Stopp“ interrupts.
                let voice = frame.speech && output == Output::Headphones;
                let _ = (frame.over_echo, playback_db);
                let conversation = conversation.clone();
                let needed = self.needed(output, frame.over_echo.is_some());
                if self.count(voice) < needed {
                    return None;
                }
                self.fired = Some(Fired {
                    route: match output {
                        Output::Headphones => Route::Headphones,
                        Output::Speakers => Route::Speakers,
                    },
                    needed,
                    echo_judged: output == Output::Speakers && frame.over_echo.is_some(),
                    level_db: Some(frame.level_db),
                    playback_db,
                });
                Some(self.hearing(conversation, Step::BargeIn))
            }
            Phase::Listening { .. } if self.other_audio => None,
            Phase::Listening { conversation, .. } => {
                let conversation = conversation.clone();
                (self.count(frame.speech) >= ONSET_FRAMES)
                    .then(|| self.hearing(conversation, Step::Heard))
            }
        }
    }

    /// What the last barge-in looked like when it fired, once.
    pub(super) fn take_fired(&mut self) -> Option<Fired> {
        self.fired.take()
    }

    /// The voice a barge-in needs from now on.
    pub(super) fn set_thresholds(&mut self, thresholds: Thresholds) {
        self.thresholds = thresholds;
    }

    /// Speech frames a barge-in needs on `output`: the speakers' count once
    /// the echo judge decides, else the headphones' (a margin over the
    /// playback level judges those frames instead).
    pub(super) fn needed(&self, output: Output, echo_judged: bool) -> u32 {
        match (output, echo_judged) {
            (Output::Speakers, true) => self.thresholds.frames(Route::Speakers),
            _ => self.thresholds.frames(Route::Headphones),
        }
    }

    /// A key, the Action shortcut, or Escape: conversation mode ends.
    pub(super) fn key(&mut self) -> Option<Step> {
        self.end()
    }

    /// The capture went away under it (a microphone loss): back to idle, with
    /// nothing left to close.
    pub(super) fn lost(&mut self) {
        self.phase = Phase::Idle;
    }

    fn end(&mut self) -> Option<Step> {
        let open = self.is_open();
        self.phase = Phase::Idle;
        open.then_some(Step::Close)
    }

    fn hearing(&mut self, conversation: ConversationId, step: Step) -> Step {
        self.phase = Phase::Hearing {
            conversation,
            quiet: 0,
            frames: self.run,
        };
        step
    }

    /// Adds a frame to the current run and returns its length.
    fn count(&mut self, speech: bool) -> u32 {
        if speech {
            self.run += 1;
            self.gap = 0;
        } else {
            self.gap += 1;
            if self.gap > RUN_GAP_FRAMES {
                self.run = 0;
            }
        }
        self.run
    }
}

/// `level` (linear, 0 to 1) in dBFS.
pub(super) fn db(level: f32) -> f32 {
    20.0 * level.max(1e-9).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Options = Options {
        listen_for: Duration::from_secs(8),
        barge_in: true,
        spoken: true,
    };

    fn speech(level_db: f32) -> Frame {
        Frame {
            speech: true,
            level_db,
            over_echo: None,
        }
    }

    const QUIET: Frame = Frame {
        speech: false,
        level_db: -70.0,
        over_echo: None,
    };

    /// Feeds `count` copies of `frame` and returns the steps they made.
    fn feed(
        machine: &mut Conversation,
        frame: Frame,
        count: u32,
        output: Output,
        playback_db: Option<f32>,
    ) -> Vec<Step> {
        (0..count)
            .filter_map(|_| machine.frame(frame, output, playback_db))
            .collect()
    }

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// An answer is read aloud, then Hex listens; speech is a follow-up in
    /// the same conversation, ended by silence.
    #[test]
    fn an_answer_then_listening_then_a_follow_up() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        let off = Options {
            barge_in: false,
            ..ON
        };
        machine.answered("c1".into(), start);
        assert_eq!(machine.tick(after(start, 100), true, false, off), None);
        assert!(
            !machine.is_open(),
            "no barge-in, no capture under the answer"
        );
        assert_eq!(machine.tick(after(start, 4_000), true, false, off), None);
        // The reading must stay over for a moment: a gap isn't its end.
        assert_eq!(machine.tick(after(start, 4_100), false, false, off), None);
        assert_eq!(
            machine.tick(after(start, 4_500), false, false, off),
            Some(Step::Listen)
        );
        assert!(machine.is_open());
        // A click or a key's sound is no question.
        assert!(feed(&mut machine, speech(-30.0), 5, Output::Speakers, None).is_empty());
        assert!(feed(&mut machine, QUIET, 3, Output::Speakers, None).is_empty());
        assert_eq!(
            feed(&mut machine, speech(-30.0), 8, Output::Speakers, None),
            [Step::Heard]
        );
        assert!(machine.is_hearing());
        // Pauses between words keep it going; the timeout no longer applies.
        assert!(feed(&mut machine, QUIET, 30, Output::Speakers, None).is_empty());
        assert_eq!(machine.tick(after(start, 60_000), false, false, off), None);
        assert!(feed(&mut machine, speech(-30.0), 20, Output::Speakers, None).is_empty());
        assert_eq!(
            feed(&mut machine, QUIET, 45, Output::Speakers, None),
            [Step::Finish("c1".into())]
        );
        assert!(!machine.is_open());
        assert_eq!(machine.tick(after(start, 70_000), false, false, off), None);
    }

    /// A gap between two sentences (the next still streaming or being
    /// made) is no end of the reading: Hex's own next sentence must not be
    /// heard as the person's follow-up.
    #[test]
    fn a_gap_between_sentences_opens_no_listening() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        let off = Options {
            barge_in: false,
            ..ON
        };
        machine.answered("c1".into(), start);
        assert_eq!(machine.tick(after(start, 100), true, false, off), None);
        // 300 ms between sentence one and two: still the answer.
        assert_eq!(machine.tick(after(start, 2_000), false, false, off), None);
        assert_eq!(machine.tick(after(start, 2_300), false, false, off), None);
        assert!(!machine.is_open());
        assert_eq!(machine.tick(after(start, 2_350), true, false, off), None);
        // The second sentence ends; only a lasting quiet ends the reading.
        assert_eq!(machine.tick(after(start, 5_000), false, false, off), None);
        assert_eq!(machine.tick(after(start, 5_390), false, false, off), None);
        assert_eq!(
            machine.tick(after(start, 5_400), false, false, off),
            Some(Step::Listen)
        );
    }

    /// The first sentence starts late (the voice is still made, or loads):
    /// while the reading is pending (`reading_aloud`), `speaking` is true and
    /// no listening opens under it.
    #[test]
    fn a_late_first_sentence_opens_no_listening() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        let off = Options {
            barge_in: false,
            ..ON
        };
        machine.answered("c1".into(), start);
        // 2.5 s of the sentence being made, past the grace for an answer
        // that is only shown.
        for ms in [100, 1_000, 1_600, 2_500] {
            assert_eq!(machine.tick(after(start, ms), true, false, off), None);
        }
        assert!(!machine.is_open());
        assert_eq!(machine.tick(after(start, 6_000), false, false, off), None);
        assert_eq!(
            machine.tick(after(start, 6_400), false, false, off),
            Some(Step::Listen)
        );
    }

    /// On speakers, with the echo of Hex's voice taken out, the person
    /// interrupts at a normal voice: no need to be louder than the answer.
    /// The echo judge is only asked on speakers, under an answer.
    #[test]
    fn the_echo_is_judged_only_under_an_answer_on_speakers() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 100), true, false, ON);
        let tone: Vec<f32> = (0..9_600).map(|i| 0.3 * (i as f32 * 0.06).sin()).collect();
        let mut asked = 0;
        machine.hear(
            &tone,
            48_000,
            Output::Speakers,
            Some(-12.0),
            &mut |frame, _| {
                asked += 1;
                assert_eq!(frame.len(), 960);
                Some(false)
            },
        );
        assert_eq!(asked, 10);
        let mut headphones = 0;
        machine.hear(&tone, 48_000, Output::Headphones, None, &mut |_, _| {
            headphones += 1;
            None
        });
        assert_eq!(headphones, 0);
    }

    /// Nothing said in `conversation_seconds`: it quietly stops.
    #[test]
    fn listening_times_out_quietly() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 10), true, false, ON);
        machine.tick(after(start, 1_600), false, false, ON);
        assert_eq!(
            machine.tick(after(start, 2_000), false, false, ON),
            Some(Step::Listen)
        );
        assert!(feed(&mut machine, QUIET, 100, Output::Headphones, None).is_empty());
        assert_eq!(machine.tick(after(start, 9_999), false, false, ON), None);
        assert_eq!(
            machine.tick(after(start, 10_000), false, false, ON),
            Some(Step::Close)
        );
        assert!(!machine.is_open());
    }

    /// An answer that's only shown starts listening at once; one that should
    /// be read but never starts, after the grace; with conversation mode off
    /// (zero seconds) it never listens.
    #[test]
    fn shown_answers_and_conversation_mode_off() {
        let start = Instant::now();
        let shown = Options {
            spoken: false,
            ..ON
        };
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(machine.tick(start, false, false, shown), Some(Step::Listen));

        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(machine.tick(after(start, 1_000), false, false, ON), None);
        assert_eq!(
            machine.tick(after(start, 1_500), false, false, ON),
            Some(Step::Listen)
        );

        let never = Options {
            listen_for: Duration::ZERO,
            barge_in: false,
            spoken: true,
        };
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 10), true, false, never);
        assert_eq!(machine.tick(after(start, 3_000), false, false, never), None);
        assert!(!machine.is_open());
    }

    /// A key, a dictation or meeting recording, or the setup unfinished
    /// ends it, closing a capture only if one is open.
    #[test]
    fn a_key_or_another_recording_ends_it() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(machine.tick(start, false, false, ON), None);
        assert_eq!(machine.key(), None, "nothing open yet");

        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 2_000), false, false, ON);
        assert_eq!(machine.key(), Some(Step::Close));
        assert_eq!(machine.key(), None);

        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 2_000), false, false, ON);
        feed(&mut machine, speech(-30.0), 8, Output::Headphones, None);
        assert!(machine.is_hearing());
        assert_eq!(
            machine.tick(after(start, 2_500), false, true, ON),
            Some(Step::Close)
        );
        assert!(!machine.is_open());

        // A lost microphone leaves nothing to close.
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 2_000), false, false, ON);
        machine.lost();
        assert_eq!(machine.key(), None);
    }

    /// With headphones, speaking over the answer interrupts it at once and
    /// becomes the follow-up; a short sound doesn't.
    #[test]
    fn barge_in_on_headphones() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(
            machine.tick(after(start, 200), true, false, ON),
            Some(Step::Watch)
        );
        assert_eq!(machine.tick(after(start, 220), true, false, ON), None);
        assert!(feed(&mut machine, speech(-40.0), 8, Output::Headphones, None).is_empty());
        assert!(feed(&mut machine, QUIET, 5, Output::Headphones, None).is_empty());
        assert_eq!(
            feed(&mut machine, speech(-40.0), 12, Output::Headphones, None),
            [Step::BargeIn]
        );
        assert!(machine.is_hearing());
        assert_eq!(
            feed(&mut machine, QUIET, 45, Output::Headphones, None),
            [Step::Finish("c1".into())]
        );
    }

    /// On speakers, the microphone hears Hex's own voice: only speech
    /// clearly louder than the answer interrupts, and without the answer's
    /// level nothing does. Once the answer ends, Hex listens as usual.
    /// A new answer while one is being listened for keeps the capture and
    /// follows the newer conversation; a follow-up being heard keeps going.
    #[test]
    fn a_newer_answer_moves_it_along() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 2_000), false, false, ON);
        machine.answered("c2".into(), after(start, 2_100));
        assert!(machine.is_open());
        assert_eq!(
            machine.tick(after(start, 4_000), false, false, ON),
            Some(Step::Listen)
        );
        feed(&mut machine, speech(-30.0), 8, Output::Speakers, None);
        machine.answered("c3".into(), after(start, 4_500));
        assert_eq!(
            feed(&mut machine, QUIET, 45, Output::Speakers, None),
            [Step::Finish("c2".into())]
        );
    }

    /// Without barge-in, a newer answer read aloud closes the capture the
    /// last one left listening; nothing hears under it, and listening
    /// starts again once it ends.
    #[test]
    fn without_barge_in_nothing_listens_under_an_answer() {
        let start = Instant::now();
        let off = Options {
            barge_in: false,
            ..ON
        };
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(
            machine.tick(
                start,
                false,
                false,
                Options {
                    spoken: false,
                    ..off
                }
            ),
            Some(Step::Listen)
        );
        machine.answered("c2".into(), after(start, 100));
        assert_eq!(
            machine.tick(after(start, 200), true, false, off),
            Some(Step::Close)
        );
        assert!(!machine.is_open());
        assert!(feed(&mut machine, speech(-10.0), 50, Output::Headphones, None).is_empty());
        machine.tick(after(start, 2_600), false, false, off);
        assert_eq!(
            machine.tick(after(start, 3_000), false, false, off),
            Some(Step::Listen)
        );
    }

    /// Microphone audio is cut into 20 ms frames at its own rate, and a
    /// loud tone over a quiet room is speech.
    #[test]
    fn audio_is_heard_in_frames() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        assert!(
            machine
                .hear(
                    &[0.5; 4_800],
                    48_000,
                    Output::Speakers,
                    None,
                    &mut |_, _| None
                )
                .is_empty()
        );
        machine.answered("c1".into(), start);
        machine.tick(
            start,
            false,
            false,
            Options {
                spoken: false,
                ..ON
            },
        );
        let quiet = vec![0.0001_f32; 48_000 / 2];
        assert!(
            machine
                .hear(&quiet, 48_000, Output::Speakers, None, &mut |_, _| None)
                .is_empty()
        );
        let tone: Vec<f32> = (0..48_000 / 2)
            .map(|i| 0.3 * (i as f32 * 0.05).sin())
            .collect();
        assert_eq!(
            machine.hear(&tone, 48_000, Output::Speakers, None, &mut |_, _| None),
            [Step::Heard]
        );
        assert!((db(0.1) + 20.0).abs() < 0.01);
        assert!(level_db(&[0.0; 960]) < -150.0);
    }

    /// Music playing when the answer ends: no hands-free window opens, so a
    /// song's lyrics never become a question.
    #[test]
    fn music_at_the_answers_end_opens_no_window() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.other_audio(true);
        assert_eq!(machine.tick(after(start, 10), true, false, ON), None);
        assert_eq!(machine.tick(after(start, 1_600), false, false, ON), None);
        assert_eq!(machine.tick(after(start, 2_000), false, false, ON), None);
        assert!(!machine.is_active());
        assert!(!machine.is_open());
        // Without music the next answer listens as usual.
        machine.other_audio(false);
        machine.answered("c2".into(), start);
        machine.tick(after(start, 2_010), true, false, ON);
        machine.tick(after(start, 3_600), false, false, ON);
        assert_eq!(
            machine.tick(after(start, 4_000), false, false, ON),
            Some(Step::Listen)
        );
    }

    /// Music starting while it listens closes the window at once, and its
    /// frames start nothing in between.
    #[test]
    fn music_starting_mid_listen_closes_it() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 10), true, false, ON);
        machine.tick(after(start, 1_600), false, false, ON);
        assert_eq!(
            machine.tick(after(start, 2_000), false, false, ON),
            Some(Step::Listen)
        );
        machine.other_audio(true);
        assert!(feed(&mut machine, speech(-20.0), 40, Output::Speakers, None).is_empty());
        assert_eq!(
            machine.tick(after(start, 2_500), false, false, ON),
            Some(Step::Close)
        );
        assert!(!machine.is_open());
    }

    /// While music plays, voice barge-in is off: nothing listens under the
    /// answer (a capture already open closes), however loud the voice.
    /// A follow-up already being heard goes on.
    #[test]
    fn music_turns_voice_barge_in_off() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(
            machine.tick(after(start, 200), true, false, ON),
            Some(Step::Watch)
        );
        machine.other_audio(true);
        assert!(feed(&mut machine, speech(-5.0), 30, Output::Headphones, None).is_empty());
        assert_eq!(
            machine.tick(after(start, 220), true, false, ON),
            Some(Step::Close)
        );
        assert!(!machine.is_open());
        assert_eq!(machine.tick(after(start, 240), true, false, ON), None);

        let mut hearing = Conversation::default();
        hearing.answered("c2".into(), start);
        hearing.tick(after(start, 10), true, false, ON);
        hearing.tick(after(start, 1_600), false, false, ON);
        hearing.tick(after(start, 2_000), false, false, ON);
        assert_eq!(
            feed(&mut hearing, speech(-30.0), 8, Output::Speakers, None),
            [Step::Heard]
        );
        hearing.other_audio(true);
        assert_eq!(hearing.tick(after(start, 2_500), false, false, ON), None);
        assert!(hearing.is_hearing());
    }

    /// On speakers a voice over the answer never interrupts, however loud
    /// or however the echo judge rules: Hex's own voice did, again and
    /// again. A hold or „Stopp“ interrupts there.
    #[test]
    fn speakers_never_barge_in() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(
            machine.tick(after(start, 100), true, false, ON),
            Some(Step::Watch)
        );
        let person = Frame {
            over_echo: Some(true),
            ..speech(0.0)
        };
        assert!(feed(&mut machine, person, 200, Output::Speakers, Some(-30.0)).is_empty());
        assert!(feed(&mut machine, speech(0.0), 200, Output::Speakers, None).is_empty());
        assert!(!machine.is_hearing());
    }
}
