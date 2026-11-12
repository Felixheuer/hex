//! What is being read aloud: the sentences in turn, the audio each engine
//! made of them, and how loud it sounds. No devices or engines here; the
//! output pulls samples and the engines feed them.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Samples per second everything is read at (mono).
pub const RATE: u32 = 48_000;
/// Below this a sample counts as silence (about −54 dB).
const QUIET: f32 = 0.002;
/// The pause kept after a sentence: engines end each with far more, which
/// reads as a robotic stop between queued sentences.
const SENTENCE_GAP: usize = RATE as usize * 120 / 1000;
/// How long the speech engine may take to send a sentence's first audio
/// before `say` reads it instead.
const FIRST_AUDIO_ENGINE: Duration = Duration::from_secs(3);
/// `say` writes a whole sentence before any of it plays.
const FIRST_AUDIO_SAY: Duration = Duration::from_secs(10);
/// A sentence whose audio stopped coming this long ago is over.
const STALL: Duration = Duration::from_secs(10);

/// What makes a sentence's audio.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    /// AVSpeechSynthesizer, through the Swift bridge.
    Bridge,
    /// `/usr/bin/say`, writing a file.
    Say,
}

/// A sentence to read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub id: u64,
    pub text: String,
    /// The voice asked for, as the caller named it.
    pub chosen: Option<String>,
}

struct Utterance {
    request: Request,
    engine: Engine,
    asked: Instant,
    last_audio: Option<Instant>,
    /// The engine sent all of it.
    done: bool,
    samples: VecDeque<f32>,
    trim: Trim,
}

/// The sentences being read, oldest first.
#[derive(Default)]
pub struct Playback {
    queue: VecDeque<Utterance>,
    since: Option<Instant>,
}

impl Playback {
    /// Queues `request` behind what is being read; `engine` makes its audio.
    pub fn add(&mut self, request: Request, engine: Engine, now: Instant) {
        self.queue.push_back(Utterance {
            request,
            engine,
            asked: now,
            last_audio: None,
            done: false,
            samples: VecDeque::new(),
            trim: Trim::default(),
        });
    }

    /// Audio `engine` made of sentence `id`; `done` when that was the last
    /// of it. Audio for a sentence no longer queued (stopped, or handed to
    /// another engine) is dropped.
    pub fn feed(&mut self, engine: Engine, id: u64, samples: &[f32], done: bool, now: Instant) {
        let Some(utterance) = self
            .queue
            .iter_mut()
            .find(|utterance| utterance.request.id == id && utterance.engine == engine)
        else {
            return;
        };
        if utterance.done {
            return;
        }
        utterance.last_audio = Some(now);
        utterance.trim.push(samples, &mut utterance.samples);
        if done {
            utterance.trim.finish(&mut utterance.samples);
            utterance.done = true;
        }
    }

    /// Fills `out` with what plays next, silence where nothing is ready.
    /// Returns how many samples were real audio.
    pub fn pull(&mut self, out: &mut [f32], now: Instant) -> usize {
        let mut written = 0;
        while written < out.len() {
            let Some(front) = self.queue.front_mut() else {
                break;
            };
            if front.samples.is_empty() {
                if front.done {
                    self.queue.pop_front();
                    continue;
                }
                // Still being made: wait for it.
                break;
            }
            let count = front.samples.len().min(out.len() - written);
            for (slot, sample) in out[written..written + count]
                .iter_mut()
                .zip(front.samples.drain(..count))
            {
                *slot = sample;
            }
            written += count;
        }
        out[written..].fill(0.0);
        if written > 0 && self.since.is_none() {
            self.since = Some(now);
        }
        while self
            .queue
            .front()
            .is_some_and(|front| front.done && front.samples.is_empty())
        {
            self.queue.pop_front();
        }
        if self.queue.is_empty() {
            self.since = None;
        }
        written
    }

    /// Drops everything queued and playing.
    pub fn stop(&mut self) {
        self.queue.clear();
        self.since = None;
    }

    /// Something is being read or waits its turn.
    pub fn busy(&self) -> bool {
        !self.queue.is_empty()
    }

    /// When the audio now playing started, if any is.
    pub fn since(&self) -> Option<Instant> {
        self.since
    }

    /// Gives up on sentences whose engine went quiet. The speech engine's
    /// that never sent audio go to `say` instead, in order, and are
    /// returned to be asked of it; `say`'s are dropped.
    pub fn expire(&mut self, now: Instant) -> Vec<Request> {
        let mut moved = Vec::new();
        let mut engine_failed = false;
        for utterance in &mut self.queue {
            if utterance.done {
                continue;
            }
            match utterance.last_audio {
                Some(last) if now.duration_since(last) > STALL => utterance.done = true,
                Some(_) => {}
                None => {
                    let waited = now.duration_since(utterance.asked);
                    match utterance.engine {
                        Engine::Bridge if engine_failed || waited > FIRST_AUDIO_ENGINE => {
                            engine_failed = true;
                            utterance.engine = Engine::Say;
                            utterance.asked = now;
                            moved.push(utterance.request.clone());
                        }
                        Engine::Say if waited > FIRST_AUDIO_SAY => utterance.done = true,
                        _ => {}
                    }
                }
            }
        }
        moved
    }
}

/// Cuts the silence an engine leaves around a sentence: all of it before
/// the first sound, and all but [`SENTENCE_GAP`] after the last. Pauses
/// inside a sentence stay.
#[derive(Default)]
struct Trim {
    started: bool,
    /// Quiet samples that may be the sentence's end.
    held: Vec<f32>,
}

impl Trim {
    fn push(&mut self, input: &[f32], out: &mut VecDeque<f32>) {
        for &sample in input {
            let quiet = sample.abs() < QUIET;
            if !self.started {
                if quiet {
                    continue;
                }
                self.started = true;
            }
            if quiet {
                self.held.push(sample);
            } else {
                out.extend(self.held.drain(..));
                out.push_back(sample);
            }
        }
    }

    fn finish(&mut self, out: &mut VecDeque<f32>) {
        let keep = self.held.len().min(SENTENCE_GAP);
        out.extend(self.held.drain(..keep));
        self.held.clear();
    }
}

/// The root-mean-square of `samples`, 0 for none.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|sample| sample * sample).sum();
    (sum / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64) -> Request {
        Request {
            id,
            text: format!("Satz {id}."),
            chosen: None,
        }
    }

    fn pull(playback: &mut Playback, count: usize, now: Instant) -> (Vec<f32>, usize) {
        let mut out = vec![9.0; count];
        let real = playback.pull(&mut out, now);
        (out, real)
    }

    #[test]
    fn sentences_play_in_turn_without_a_gap() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Bridge, now);
        playback.add(request(2), Engine::Bridge, now);
        // The second sentence's audio comes first: it waits its turn.
        playback.feed(Engine::Bridge, 2, &[0.5, 0.5], true, now);
        assert_eq!(pull(&mut playback, 2, now), (vec![0.0, 0.0], 0));
        assert!(playback.busy() && playback.since().is_none());
        playback.feed(Engine::Bridge, 1, &[0.1, 0.2], false, now);
        playback.feed(Engine::Bridge, 1, &[0.3], true, now);
        let later = now + Duration::from_millis(5);
        assert_eq!(
            pull(&mut playback, 6, later),
            (vec![0.1, 0.2, 0.3, 0.5, 0.5, 0.0], 5)
        );
        // All of it played: idle again.
        assert!(!playback.busy());
        assert_eq!(playback.since(), None);
    }

    #[test]
    fn speaking_since_is_when_the_audio_started() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Bridge, now);
        pull(&mut playback, 4, now);
        assert_eq!(playback.since(), None, "nothing played yet");
        playback.feed(Engine::Bridge, 1, &[0.5; 8], true, now);
        let started = now + Duration::from_millis(30);
        pull(&mut playback, 4, started);
        pull(&mut playback, 2, started + Duration::from_millis(10));
        assert_eq!(playback.since(), Some(started));
        pull(&mut playback, 4, started + Duration::from_millis(20));
        assert_eq!(playback.since(), None);
    }

    #[test]
    fn stop_drops_the_queue_and_late_audio() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Bridge, now);
        playback.add(request(2), Engine::Bridge, now);
        playback.feed(Engine::Bridge, 1, &[0.5; 100], false, now);
        pull(&mut playback, 10, now);
        playback.stop();
        assert!(!playback.busy());
        assert_eq!(playback.since(), None);
        playback.feed(Engine::Bridge, 1, &[0.5; 100], true, now);
        playback.feed(Engine::Bridge, 2, &[0.5; 100], true, now);
        assert_eq!(pull(&mut playback, 10, now).1, 0);
        assert!(!playback.busy());
    }

    #[test]
    fn silence_around_a_sentence_is_cut_to_a_short_pause() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Say, now);
        let mut audio = vec![0.0; 5_000];
        audio.extend([0.5, 0.0005, 0.4]);
        audio.extend(vec![0.0; SENTENCE_GAP * 3]);
        // Audio for a sentence `say` isn't reading is not this one's.
        playback.feed(Engine::Bridge, 1, &[0.9; 10], true, now);
        playback.feed(Engine::Say, 1, &audio, true, now);
        let mut out = vec![0.0; SENTENCE_GAP * 4];
        let real = playback.pull(&mut out, now);
        assert_eq!(real, 3 + SENTENCE_GAP);
        assert_eq!(out[..3], [0.5, 0.0005, 0.4]);
    }

    #[test]
    fn a_silent_engine_hands_its_sentences_to_say() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Bridge, now);
        playback.add(request(2), Engine::Bridge, now + Duration::from_secs(2));
        assert!(playback.expire(now + Duration::from_secs(2)).is_empty());
        let moved = playback.expire(now + Duration::from_millis(3_100));
        assert_eq!(moved, vec![request(1), request(2)]);
        // The engine's late audio is ignored; `say`'s plays.
        let later = now + Duration::from_secs(4);
        playback.feed(Engine::Bridge, 1, &[0.9; 4], true, later);
        playback.feed(Engine::Say, 1, &[0.3; 4], true, later);
        assert_eq!(pull(&mut playback, 4, later), (vec![0.3; 4], 4));
        // `say` too stays quiet: the sentence is dropped.
        assert!(playback.expire(later + Duration::from_secs(11)).is_empty());
        assert_eq!(pull(&mut playback, 4, later).1, 0);
        assert!(!playback.busy());
    }

    #[test]
    fn a_stalled_sentence_ends() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.add(request(1), Engine::Bridge, now);
        playback.feed(Engine::Bridge, 1, &[0.5; 4], false, now);
        assert!(playback.expire(now + Duration::from_secs(20)).is_empty());
        pull(&mut playback, 8, now);
        assert!(!playback.busy());
    }

    #[test]
    fn rms_is_the_root_mean_square() {
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert!((rms(&[0.0, 1.0, 0.0, 1.0]) - 0.5_f32.sqrt()).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }
}
