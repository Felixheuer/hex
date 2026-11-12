//! Plays the sentences [`Playback`] holds on the default output, and
//! measures how loud they are. The device opens when something is to be
//! read and closes a few seconds after the last sentence, so an idle Hex
//! holds no output open.

use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rodio::cpal::BufferSize;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Source};

use super::playback::{self, Playback, RATE};

/// Samples the output takes from the queue at a time (10 ms).
const CHUNK: usize = RATE as usize / 100;
/// A stop fades the audio already taken out over this many samples (4 ms)
/// instead of clicking.
const FADE: usize = RATE as usize / 250;
/// The device's buffer: about 11 ms at 48 kHz, so a stop is heard at once.
const DEVICE_FRAMES: u32 = 512;
/// How long the device stays open after the last sentence, for the next.
const IDLE_CLOSE: Duration = Duration::from_secs(4);
/// How long after a failed open the next try waits.
const RETRY: Duration = Duration::from_secs(2);

/// What the output, the engines, and the callers share.
#[derive(Default)]
pub struct Shared {
    playback: Mutex<Playback>,
    wake: Condvar,
    /// Bumped by every new sentence, so the output thread wakes for it.
    asked: AtomicU64,
    /// Bumped by every stop, so the audio thread drops what it holds.
    cut: AtomicU64,
    /// The RMS of the last [`WINDOW`] chunks played (f32 bits).
    level: AtomicU32,
}

impl Shared {
    pub fn playback(&self) -> MutexGuard<'_, Playback> {
        self.playback
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Wakes the output thread for a new sentence.
    pub fn asked(&self) {
        self.asked.fetch_add(1, Ordering::AcqRel);
        self.wake.notify_all();
    }

    /// Stops what plays, at once.
    pub fn stop(&self) {
        self.playback().stop();
        self.cut.fetch_add(1, Ordering::AcqRel);
    }

    /// The linear RMS (0..1) of the last 30 ms played.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Acquire))
    }
}

/// Chunks the level averages: 30 ms, so it follows syllables, not samples.
const WINDOW: usize = 3;

/// The level over the last [`WINDOW`] chunks: the root of their mean
/// squares' mean.
#[derive(Default)]
struct Window {
    squares: [f32; WINDOW],
    next: usize,
}

impl Window {
    /// Adds a chunk; returns the level with it.
    fn push(&mut self, chunk: &[f32]) -> f32 {
        self.squares[self.next] = playback::rms(chunk).powi(2);
        self.next = (self.next + 1) % WINDOW;
        (self.squares.iter().sum::<f32>() / WINDOW as f32)
            .sqrt()
            .clamp(0.0, 1.0)
    }
}

/// The audio thread's view of the queue: takes 10 ms at a time and plays
/// them, silence when nothing is ready. Never ends; the device closing
/// drops it.
struct Reader {
    shared: Arc<Shared>,
    chunk: Vec<f32>,
    at: usize,
    cut: u64,
    /// Samples left to fade out after a stop.
    fade: usize,
    window: Window,
}

impl Reader {
    fn new(shared: Arc<Shared>) -> Self {
        let cut = shared.cut.load(Ordering::Acquire);
        Self {
            shared,
            chunk: Vec::with_capacity(CHUNK),
            at: 0,
            cut,
            fade: 0,
            window: Window::default(),
        }
    }

    fn refill(&mut self) {
        self.chunk.clear();
        self.chunk.resize(CHUNK, 0.0);
        self.at = 0;
        self.fade = 0;
        self.shared.playback().pull(&mut self.chunk, Instant::now());
        // What Hex plays, for conversation mode to tell its echo from the
        // person (`recognition::echo`).
        crate::audio::playback_reference::played(
            &self.chunk,
            RATE,
            crate::audio::instant::CaptureInstant::now(),
        );
        let level = self.window.push(&self.chunk);
        self.shared.level.store(level.to_bits(), Ordering::Release);
    }
}

impl Iterator for Reader {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let cut = self.shared.cut.load(Ordering::Acquire);
        if cut != self.cut {
            self.cut = cut;
            self.fade = FADE.min(self.chunk.len() - self.at);
            self.chunk.truncate(self.at + self.fade);
        }
        if self.at >= self.chunk.len() {
            self.refill();
        }
        let mut sample = self.chunk[self.at];
        if self.fade > 0 {
            sample *= (self.chunk.len() - self.at) as f32 / self.fade as f32;
        }
        self.at += 1;
        Some(sample)
    }
}

impl Source for Reader {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::<u16>::MIN
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(RATE).unwrap_or(NonZero::<u32>::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct Opened {
    _sink: MixerDeviceSink,
    /// Set by the stream's error callback (on a CoreAudio thread, so it only
    /// flips this).
    failed: Arc<AtomicBool>,
}

fn open(shared: &Arc<Shared>) -> color_eyre::Result<Opened> {
    use color_eyre::eyre::WrapErr;
    let failed = Arc::new(AtomicBool::new(false));
    let callback = {
        let failed = failed.clone();
        move |error: rodio::cpal::Error| {
            if !matches!(
                error.kind(),
                rodio::cpal::ErrorKind::Xrun | rodio::cpal::ErrorKind::DeviceChanged
            ) {
                failed.store(true, Ordering::Release);
            }
        }
    };
    let mut sink = DeviceSinkBuilder::from_default_device()
        .and_then(|builder| {
            builder
                .with_buffer_size(BufferSize::Fixed(DEVICE_FRAMES))
                .with_error_callback(callback)
                .open_stream()
        })
        .or_else(|_| DeviceSinkBuilder::open_default_sink())
        .wrap_err("could not open the audio output")?;
    sink.log_on_drop(false);
    sink.mixer().add(Reader::new(shared.clone()));
    Ok(Opened {
        _sink: sink,
        failed,
    })
}

/// The output thread: opens the device while something is to be read,
/// follows the default output when it changes, and closes the device when
/// idle. `tick` runs first on every round and says whether anything is
/// queued.
pub fn run(shared: Arc<Shared>, tick: impl Fn() -> bool) {
    let mut output: Option<Opened> = None;
    let mut route = 0;
    let mut retry_at = Instant::now();
    let mut idle_since: Option<Instant> = None;
    loop {
        let seen = shared.asked.load(Ordering::Acquire);
        let busy = tick();
        let now = Instant::now();
        if busy {
            idle_since = None;
            let current = crate::audio::output_route_generation();
            if output
                .as_ref()
                .is_some_and(|opened| opened.failed.load(Ordering::Acquire) || route != current)
            {
                tracing::debug!("reopening the speech output");
                output = None;
            }
            if output.is_none() && now >= retry_at {
                route = current;
                match open(&shared) {
                    Ok(opened) => output = Some(opened),
                    Err(error) => {
                        tracing::warn!(%error, "could not read aloud");
                        retry_at = now + RETRY;
                        // Nothing could play it: don't keep callers waiting.
                        shared.stop();
                    }
                }
            }
        } else if output.is_some()
            && now.duration_since(*idle_since.get_or_insert(now)) >= IDLE_CLOSE
        {
            output = None;
        }
        let wait = if busy {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        };
        let guard = shared.playback();
        let _ = shared.wake.wait_timeout_while(guard, wait, |_| {
            shared.asked.load(Ordering::Acquire) == seen
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::speech::playback::{Engine, Request};

    #[test]
    fn the_reader_plays_the_queue_and_fades_out_on_stop() {
        let shared = Arc::new(Shared::default());
        let now = Instant::now();
        {
            let mut playback = shared.playback();
            playback.add(
                Request {
                    id: 1,
                    text: "Hallo.".into(),
                    chosen: None,
                },
                Engine::Bridge,
                now,
            );
            playback.feed(Engine::Bridge, 1, &vec![0.5; CHUNK * 3], true, now);
        }
        let mut reader = Reader::new(shared.clone());
        assert!(reader.by_ref().take(10).all(|sample| sample == 0.5));
        // One chunk of the window played at 0.5.
        assert!((shared.level() - 0.5 / 3_f32.sqrt()).abs() < 1e-5);
        shared.stop();
        let faded: Vec<f32> = reader.by_ref().take(FADE + 10).collect();
        assert!(faded[0] <= 0.5 && faded[0] > 0.49);
        assert!(
            faded
                .windows(2)
                .take(FADE - 1)
                .all(|pair| pair[1] < pair[0])
        );
        assert!(faded[FADE..].iter().all(|sample| *sample == 0.0));
        assert!(!shared.playback().busy());
    }

    #[test]
    fn the_level_averages_the_last_30_ms() {
        let mut window = Window::default();
        assert_eq!(window.push(&[0.0; CHUNK]), 0.0);
        let one = window.push(&[0.6; CHUNK]);
        assert!((one - (0.36_f32 / 3.0).sqrt()).abs() < 1e-5);
        window.push(&[0.6; CHUNK]);
        let full = window.push(&[0.6; CHUNK]);
        assert!((full - 0.6).abs() < 1e-5);
        // Quiet again: gone within the window.
        window.push(&[0.0; CHUNK]);
        window.push(&[0.0; CHUNK]);
        assert_eq!(window.push(&[0.0; CHUNK]), 0.0);
        assert_eq!(window.push(&[4.0; CHUNK]), 1.0);
    }
}
