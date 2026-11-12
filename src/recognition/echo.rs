//! Whether the person speaks over Hex's own voice on the speakers (barge-in
//! without headphones). Hex knows exactly what it plays
//! (`audio::playback_reference`), so each 20 ms of microphone audio is
//! compared, band by band, with what Hex played a moment earlier:
//!
//! - the delay from output to microphone (output buffer, the room, the
//!   input buffer) is found among 0 to 300 ms as the one whose loudness
//!   follows the microphone's best;
//! - each band's echo path (how much of Hex's voice reaches the microphone)
//!   is a low percentile ([`PATH_PERCENTILE`]) of its microphone-to-reference
//!   ratio over the last two seconds: mostly Hex's echo alone, so the
//!   person's voice (which only raises the ratio) doesn't count, and a few
//!   frames before the echo arrived don't hold it down;
//! - what the echo can't explain is the person. A frame is the person when
//!   at least [`OVER_SHARE`] of its energy is left after taking out the
//!   echo (with headroom, [`OVER_SUBTRACT`]).
//!
//! A double-talk detector on band energies rather than an adaptive filter on
//! samples: a laptop's speakers distort, which a linear filter can't
//! cancel, and barge-in only needs the decision, not clean audio (Hex stops
//! reading at once, so the follow-up itself is heard without echo). Pure:
//! the reference comes from a closure, so tests feed synthetic signals.

use std::sync::Arc;
use std::time::Duration;

use realfft::{RealFftPlanner, RealToComplex};

/// Bands, log-spaced over speech (150 Hz to 7 kHz, or half the rate).
const BANDS: usize = 16;
const LOW_HZ: f32 = 150.0;
const HIGH_HZ: f32 = 7_000.0;
/// Delays tried, every 20 ms from 0 to 300 ms.
const DELAYS: usize = 16;
const DELAY_STEP: Duration = Duration::from_millis(20);
/// Frames each delay's fit is scored over: one second.
const SCORE_FRAMES: usize = 50;
/// Frames with Hex's voice it needs before it trusts its echo path.
const SETTLE_FRAMES: u32 = 15;
/// The echo is taken out this many times over, for its variation.
const OVER_SUBTRACT: f32 = 1.5;
/// Share of a frame's energy left after the echo that makes it the person.
const OVER_SHARE: f32 = 0.35;
/// Frames of ratios each band's echo path is taken from: two seconds.
const PATH_FRAMES: usize = 100;
/// The percentile of them that is the echo path.
const PATH_PERCENTILE: f32 = 0.2;
/// Reference energy below which a band counts as silent.
const QUIET: f32 = 1e-9;

/// What a frame holds besides Hex's echo.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Verdict {
    /// Hex played nothing lately: there is no echo, as with headphones.
    NoEcho,
    /// Hex plays, but its echo path isn't learnt yet.
    Unsure,
    /// The person speaks over Hex's voice (or not).
    OverEcho(bool),
}

/// The detector for one capture. Holds FFT plans for its rate.
pub(super) struct EchoGate {
    rate: u32,
    size: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    /// Each band's FFT bins.
    bands: Vec<std::ops::Range<usize>>,
    window: Vec<f32>,
    /// Recent microphone-to-reference ratios per band.
    ratios: Vec<std::collections::VecDeque<f32>>,
    /// Frames with Hex's voice seen.
    settled: u32,
    /// Per delay: recent (log mic energy, log reference energy) pairs.
    history: Vec<std::collections::VecDeque<(f32, f32)>>,
    delay: usize,
}

impl EchoGate {
    pub(super) fn new(rate: u32) -> Self {
        let size = (rate / 50).max(64) as usize;
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(size);
        let bin_hz = rate as f32 / size as f32;
        let high = HIGH_HZ.min(rate as f32 / 2.0 - bin_hz);
        let edges: Vec<usize> = (0..=BANDS)
            .map(|band| {
                let hz = LOW_HZ * (high / LOW_HZ).powf(band as f32 / BANDS as f32);
                ((hz / bin_hz).round() as usize).clamp(1, size / 2)
            })
            .collect();
        let bands = edges
            .windows(2)
            .map(|pair| pair[0]..pair[1].max(pair[0] + 1))
            .collect();
        let window = (0..size)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / size as f32).cos())
            .collect();
        Self {
            rate,
            size,
            fft,
            bands,
            window,
            ratios: vec![std::collections::VecDeque::new(); BANDS],
            settled: 0,
            history: vec![std::collections::VecDeque::new(); DELAYS],
            delay: 0,
        }
    }

    pub(super) fn rate(&self) -> u32 {
        self.rate
    }

    /// The samples a frame holds.
    pub(super) fn frame_len(&self) -> usize {
        self.size
    }

    /// The delay it found, from output to microphone.
    #[cfg(test)]
    fn delay(&self) -> Duration {
        DELAY_STEP * self.delay as u32
    }

    /// Judges one frame of microphone audio. `reference(delay)` is what Hex
    /// played over the frame's span, `delay` earlier (frame-long, at this
    /// rate), or `None` when it played nothing lately.
    pub(super) fn judge(
        &mut self,
        mic: &[f32],
        reference: &dyn Fn(Duration) -> Option<Vec<f32>>,
    ) -> Verdict {
        if mic.len() != self.size {
            return Verdict::Unsure;
        }
        let mic_bands = self.band_energies(mic);
        let mut candidates = Vec::with_capacity(DELAYS);
        for delay in 0..DELAYS {
            let Some(played) = reference(DELAY_STEP * delay as u32) else {
                if delay == 0 {
                    return Verdict::NoEcho;
                }
                candidates.push(None);
                continue;
            };
            candidates.push((played.len() == self.size).then(|| self.band_energies(&played)));
        }
        let mic_total: f32 = mic_bands.iter().sum();
        for (delay, candidate) in candidates.iter().enumerate() {
            let Some(bands) = candidate else { continue };
            let total: f32 = bands.iter().sum();
            if total <= QUIET * BANDS as f32 {
                continue;
            }
            let history = &mut self.history[delay];
            history.push_back((log(mic_total), log(total)));
            if history.len() > SCORE_FRAMES {
                history.pop_front();
            }
        }
        self.delay = best_delay(&self.history).unwrap_or(self.delay);
        let Some(Some(played)) = candidates.get(self.delay) else {
            return Verdict::Unsure;
        };
        let played_total: f32 = played.iter().sum();
        if played_total <= QUIET * BANDS as f32 {
            // Hex is between words: whatever is heard is the person's.
            return if self.settled >= SETTLE_FRAMES {
                Verdict::OverEcho(true)
            } else {
                Verdict::Unsure
            };
        }
        let mut left = 0.0;
        for band in 0..BANDS {
            let echo = played[band];
            if echo <= QUIET {
                left += mic_bands[band];
                continue;
            }
            let ratios = &mut self.ratios[band];
            ratios.push_back(mic_bands[band] / echo);
            if ratios.len() > PATH_FRAMES {
                ratios.pop_front();
            }
            let gain = percentile(ratios, PATH_PERCENTILE);
            left += (mic_bands[band] - OVER_SUBTRACT * gain * echo).max(0.0);
        }
        self.settled = self.settled.saturating_add(1);
        if self.settled < SETTLE_FRAMES {
            return Verdict::Unsure;
        }
        Verdict::OverEcho(mic_total > 0.0 && left >= OVER_SHARE * mic_total)
    }

    fn band_energies(&self, samples: &[f32]) -> [f32; BANDS] {
        let mut input: Vec<f32> = samples
            .iter()
            .zip(&self.window)
            .map(|(sample, weight)| sample * weight)
            .collect();
        let mut spectrum = self.fft.make_output_vec();
        let mut bands = [0.0; BANDS];
        if self.fft.process(&mut input, &mut spectrum).is_err() {
            return bands;
        }
        for (band, bins) in self.bands.iter().enumerate() {
            bands[band] = spectrum[bins.clone()]
                .iter()
                .map(|bin| bin.norm_sqr())
                .sum::<f32>()
                / self.size as f32;
        }
        bands
    }
}

/// The `share` percentile of `values`.
fn percentile(values: &std::collections::VecDeque<f32>, share: f32) -> f32 {
    let mut sorted: Vec<f32> = values.iter().copied().collect();
    sorted.sort_by(f32::total_cmp);
    let index = ((sorted.len() as f32 - 1.0) * share).round() as usize;
    sorted.get(index).copied().unwrap_or(0.0)
}

fn log(energy: f32) -> f32 {
    energy.max(1e-12).ln()
}

/// The delay whose reference loudness correlates best with the microphone's
/// over the recent frames; `None` until one has enough frames to tell.
fn best_delay(history: &[std::collections::VecDeque<(f32, f32)>]) -> Option<usize> {
    history
        .iter()
        .enumerate()
        .filter(|(_, pairs)| pairs.len() >= 10)
        .map(|(delay, pairs)| (delay, correlation(pairs)))
        .filter(|(_, score)| score.is_finite())
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(delay, _)| delay)
}

fn correlation(pairs: &std::collections::VecDeque<(f32, f32)>) -> f32 {
    let count = pairs.len() as f32;
    let (mean_a, mean_b) = pairs
        .iter()
        .fold((0.0, 0.0), |(a, b), (x, y)| (a + x / count, b + y / count));
    let (mut cov, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
    for (x, y) in pairs {
        cov += (x - mean_a) * (y - mean_b);
        var_a += (x - mean_a).powi(2);
        var_b += (y - mean_b).powi(2);
    }
    cov / (var_a * var_b).sqrt().max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;
    const FRAME: usize = 320;

    /// A speech-like signal: noise whose loudness changes every 60 ms (as
    /// syllables do), shaped by a slowly moving pair of tones.
    fn voice(seed: u64, samples: usize, pitch: f32) -> Vec<f32> {
        let mut state = seed;
        let mut noise = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            ((state >> 33) as f32 / (1u64 << 31) as f32) - 1.0
        };
        let mut syllable = 1.0;
        (0..samples)
            .map(|i| {
                if i % 960 == 0 {
                    syllable = 0.2 + (noise() + 1.0) * 0.6;
                }
                let t = i as f32 / RATE as f32;
                let tone = (2.0 * std::f32::consts::PI * pitch * t).sin()
                    + 0.5 * (2.0 * std::f32::consts::PI * pitch * 3.1 * t).sin();
                syllable * (0.2 * tone + 0.1 * noise())
            })
            .collect()
    }

    /// Runs `mic` (with Hex's `played` voice reaching it after `delay`
    /// frames) through a gate; the verdicts per frame.
    fn run(played: &[f32], mic: &[f32]) -> Vec<Verdict> {
        let mut gate = EchoGate::new(RATE);
        let frames = mic.len() / FRAME;
        (0..frames)
            .map(|frame| {
                let end = (frame + 1) * FRAME;
                let reference = |delay: Duration| {
                    let back = (delay.as_secs_f64() * f64::from(RATE)) as usize;
                    let stop = end.checked_sub(back)?;
                    let start = stop.checked_sub(FRAME)?;
                    Some(played[start..stop].to_vec())
                };
                gate.judge(&mic[frame * FRAME..end], &reference)
            })
            .collect()
    }

    fn echo_of(played: &[f32], delay_samples: usize, gain: f32) -> Vec<f32> {
        (0..played.len())
            .map(|i| {
                i.checked_sub(delay_samples)
                    .map_or(0.0, |j| played[j] * gain)
            })
            .collect()
    }

    #[test]
    fn hexs_own_voice_alone_is_no_barge_in() {
        let played = voice(1, RATE as usize * 4, 220.0);
        // 60 ms to the microphone, a fifth as loud, plus a little room noise.
        let room = voice(9, played.len(), 50.0);
        let mic: Vec<f32> = echo_of(&played, 960, 0.2)
            .iter()
            .zip(&room)
            .map(|(echo, noise)| echo + 0.002 * noise)
            .collect();
        let verdicts = run(&played, &mic);
        let late = &verdicts[60..];
        let over = late
            .iter()
            .filter(|verdict| **verdict == Verdict::OverEcho(true))
            .count();
        assert!(
            over * 10 < late.len(),
            "{over} of {} frames taken for the person",
            late.len()
        );
    }

    #[test]
    fn the_person_talking_over_hex_is_heard() {
        let played = voice(1, RATE as usize * 4, 220.0);
        let person = voice(5, played.len(), 130.0);
        let echo = echo_of(&played, 960, 0.2);
        // The person starts after two seconds, about as loud as the echo.
        let mic: Vec<f32> = echo
            .iter()
            .zip(&person)
            .enumerate()
            .map(|(i, (echo, person))| {
                echo + if i >= RATE as usize * 2 {
                    0.4 * person
                } else {
                    0.0
                }
            })
            .collect();
        let verdicts = run(&played, &mic);
        let before = &verdicts[60..100];
        let during = &verdicts[110..];
        let over = |frames: &[Verdict]| {
            frames
                .iter()
                .filter(|verdict| **verdict == Verdict::OverEcho(true))
                .count()
        };
        assert!(over(before) * 10 < before.len(), "echo alone");
        assert!(
            over(during) * 2 > during.len(),
            "{} of {} frames heard as the person",
            over(during),
            during.len()
        );
    }

    #[test]
    fn it_finds_the_delay_to_the_microphone() {
        let played = voice(3, RATE as usize * 3, 200.0);
        let mic = echo_of(&played, 1_920, 0.3); // 120 ms
        let mut gate = EchoGate::new(RATE);
        for frame in 0..(mic.len() / FRAME) {
            let end = (frame + 1) * FRAME;
            let reference = |delay: Duration| {
                let back = (delay.as_secs_f64() * f64::from(RATE)) as usize;
                let stop = end.checked_sub(back)?;
                let start = stop.checked_sub(FRAME)?;
                Some(played[start..stop].to_vec())
            };
            gate.judge(&mic[frame * FRAME..end], &reference);
        }
        assert_eq!(gate.delay(), Duration::from_millis(120));
    }

    #[test]
    fn without_hex_playing_there_is_no_echo() {
        let mut gate = EchoGate::new(RATE);
        let verdict = gate.judge(&vec![0.1; FRAME], &|_| None);
        assert_eq!(verdict, Verdict::NoEcho);
        assert_eq!(gate.frame_len(), FRAME);
        assert_eq!(gate.rate(), RATE);
    }
}
