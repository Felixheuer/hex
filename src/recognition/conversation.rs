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
//!     │       headphones: any speech; speakers: speech louder than the answer
//!     │       itself (speech::level() + 6 dB), never without that level
//!     └── the reading ends (or never starts within 1.5 s, or answers aren't
//!         read aloud) -> Step::Listen -> Listening(c) for conversation_seconds
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
use crate::transcription::pause_segmenter::SpeechFrames;

/// How long an answer may wait to start being read aloud before Hex counts
/// it as shown only.
const SPEAK_GRACE: Duration = Duration::from_millis(1_500);
/// Speech frames in a run that start a follow-up: 160 ms, the speech
/// detector's shortest speech.
const ONSET_FRAMES: u32 = 8;
/// Speech frames in a run that interrupt an answer: 240 ms.
const BARGE_IN_FRAMES: u32 = 12;
/// Quiet frames a run forgives.
const RUN_GAP_FRAMES: u32 = 2;
/// Quiet frames that end a follow-up: 900 ms.
const END_FRAMES: u32 = 45;
/// The longest follow-up: 60 s.
const MAX_FRAMES: u32 = 3_000;
/// On speakers, how far a frame must be above the answer's own playback
/// level to be the person rather than Hex's voice.
const SPEAKER_MARGIN_DB: f32 = 6.0;
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
}

/// Cuts microphone audio into 20 ms frames and classifies them.
#[derive(Default)]
struct Ears {
    frame: Vec<f32>,
    speech: SpeechFrames,
}

impl Ears {
    fn push(&mut self, samples: &[f32], rate: u32, mut each: impl FnMut(Frame)) {
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
}

impl Default for Conversation {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ears: Ears::default(),
            other_audio: false,
            run: 0,
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
        };
    }

    /// Moves on with time: `speaking` is whether Hex reads aloud now;
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
            } => {
                if speaking {
                    *spoke = true;
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
                let over = *spoke || !options.spoken || now.duration_since(*since) >= SPEAK_GRACE;
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
    /// if known.
    pub(super) fn hear(
        &mut self,
        samples: &[f32],
        rate: u32,
        output: Output,
        playback_db: Option<f32>,
    ) -> Vec<Step> {
        let mut steps = Vec::new();
        if !self.is_open() {
            return steps;
        }
        let mut frames = Vec::new();
        self.ears.push(samples, rate, |frame| frames.push(frame));
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
                let voice = frame.speech
                    && match output {
                        Output::Headphones => true,
                        Output::Speakers => playback_db
                            .is_some_and(|playback| frame.level_db > playback + SPEAKER_MARGIN_DB),
                    };
                let conversation = conversation.clone();
                (self.count(voice) >= BARGE_IN_FRAMES)
                    .then(|| self.hearing(conversation, Step::BargeIn))
            }
            Phase::Listening { .. } if self.other_audio => None,
            Phase::Listening { conversation, .. } => {
                let conversation = conversation.clone();
                (self.count(frame.speech) >= ONSET_FRAMES)
                    .then(|| self.hearing(conversation, Step::Heard))
            }
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
        }
    }

    const QUIET: Frame = Frame {
        speech: false,
        level_db: -70.0,
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
        assert_eq!(
            machine.tick(after(start, 4_100), false, false, off),
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

    /// Nothing said in `conversation_seconds`: it quietly stops.
    #[test]
    fn listening_times_out_quietly() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        machine.tick(after(start, 10), true, false, ON);
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
    #[test]
    fn barge_in_on_speakers_needs_a_voice_over_the_answer() {
        let start = Instant::now();
        let mut machine = Conversation::default();
        machine.answered("c1".into(), start);
        assert_eq!(
            machine.tick(after(start, 200), true, false, ON),
            Some(Step::Watch)
        );
        // Hex's own voice through the speakers, as loud as it plays or a
        // little louder: never a barge-in.
        assert!(
            feed(
                &mut machine,
                speech(-20.0),
                100,
                Output::Speakers,
                Some(-20.0)
            )
            .is_empty()
        );
        assert!(
            feed(
                &mut machine,
                speech(-15.0),
                100,
                Output::Speakers,
                Some(-20.0)
            )
            .is_empty()
        );
        // Without the playback level, speakers never barge in.
        assert!(feed(&mut machine, speech(0.0), 100, Output::Speakers, None).is_empty());
        // The person, clearly louder than the answer.
        assert_eq!(
            feed(
                &mut machine,
                speech(-10.0),
                12,
                Output::Speakers,
                Some(-20.0)
            ),
            [Step::BargeIn]
        );

        // Not interrupted: when the reading ends the open capture listens.
        let mut machine = Conversation::default();
        machine.answered("c2".into(), start);
        machine.tick(after(start, 200), true, false, ON);
        assert_eq!(
            machine.tick(after(start, 3_000), false, false, ON),
            Some(Step::Listen)
        );
        assert_eq!(
            feed(&mut machine, speech(-30.0), 8, Output::Speakers, None),
            [Step::Heard]
        );
    }

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
                .hear(&[0.5; 4_800], 48_000, Output::Speakers, None)
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
                .hear(&quiet, 48_000, Output::Speakers, None)
                .is_empty()
        );
        let tone: Vec<f32> = (0..48_000 / 2)
            .map(|i| 0.3 * (i as f32 * 0.05).sin())
            .collect();
        assert_eq!(
            machine.hear(&tone, 48_000, Output::Speakers, None),
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
        assert_eq!(machine.tick(after(start, 2_000), false, false, ON), None);
        assert!(!machine.is_active());
        assert!(!machine.is_open());
        // Without music the next answer listens as usual.
        machine.other_audio(false);
        machine.answered("c2".into(), start);
        machine.tick(after(start, 2_010), true, false, ON);
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
        hearing.tick(after(start, 2_000), false, false, ON);
        assert_eq!(
            feed(&mut hearing, speech(-30.0), 8, Output::Speakers, None),
            [Step::Heard]
        );
        hearing.other_audio(true);
        assert_eq!(hearing.tick(after(start, 2_500), false, false, ON), None);
        assert!(hearing.is_hearing());
    }
}
