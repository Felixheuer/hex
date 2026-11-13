//! The fallback engine on macOS and the engine on Linux and Windows: the
//! system's speech program writes each sentence to WAV audio, which plays
//! through the same output as the speech engine's, so stopping, queueing,
//! and the level work the same.
//!
//! ```ts
//! macOS    /usr/bin/say                       (the system voices; AVSpeech reads first)
//! Linux    piper + a downloaded voice          (natural; ~/.local/share/piper-voices, …)
//!          pico2wave                           (de, en, es, fr, it)
//!          espeak-ng / espeak                  (every language, robotic)
//! Windows  Windows.Media.SpeechSynthesis       (the installed Windows voices, in process)
//! ```

use std::io::Write;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use super::playback::RATE;

#[cfg(target_os = "linux")]
#[path = "say/linux.rs"]
mod system;
#[cfg(target_os = "macos")]
#[path = "say/macos.rs"]
mod system;
#[cfg(target_os = "windows")]
#[path = "say/windows.rs"]
mod system;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[path = "say/none.rs"]
mod system;

pub use system::voices;

/// How one sentence's audio is made.
pub(super) enum Plan {
    /// A program that writes the WAV file it was given, reading the text
    /// from its standard input when `stdin` holds it.
    Command {
        command: std::process::Command,
        stdin: Option<String>,
    },
    /// The audio, made in process (a WAV file's bytes), or `None` when it
    /// couldn't be.
    Audio(Option<Vec<u8>>),
}

/// A sentence for `say`.
pub struct Job {
    pub id: u64,
    pub text: String,
    /// A name `say -v` knows.
    pub voice: Option<String>,
}

/// Sentence audio, or `None` when `say` couldn't make it.
pub type Deliver = fn(u64, Option<Vec<f32>>);

struct Worker {
    jobs: Sender<(u64, Job)>,
}

/// The `say` writing now, so a stop can end it.
static CURRENT: Mutex<Option<Child>> = Mutex::new(None);
/// Bumped by a stop: queued sentences from before it are skipped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn worker(deliver: Deliver) -> Option<&'static Worker> {
    static WORKER: OnceLock<Option<Worker>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (jobs, receiver) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("hex-speech-say".into())
                .spawn(move || work(&receiver, deliver))
                .map_err(|error| tracing::warn!(%error, "could not start reading aloud"))
                .ok()?;
            Some(Worker { jobs })
        })
        .as_ref()
}

/// Asks `say` for `job`'s audio, after the sentences before it.
pub fn speak(job: Job, deliver: Deliver) {
    let id = job.id;
    let sent = worker(deliver).is_some_and(|worker| {
        worker
            .jobs
            .send((GENERATION.load(Ordering::Acquire), job))
            .is_ok()
    });
    if !sent {
        deliver(id, None);
    }
}

/// Drops every queued sentence and ends the one being written.
pub fn stop() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(mut child) = CURRENT
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        crate::assistant::backends::run::kill_group(&mut child);
    }
}

fn work(jobs: &Receiver<(u64, Job)>, deliver: Deliver) {
    while let Ok((generation, job)) = jobs.recv() {
        if generation != GENERATION.load(Ordering::Acquire) {
            continue;
        }
        let audio = write(&job, generation);
        if generation == GENERATION.load(Ordering::Acquire) {
            deliver(job.id, audio);
        }
    }
}

/// Has the system's speech program write `job` to a file and reads it back.
fn write(job: &Job, generation: u64) -> Option<Vec<f32>> {
    let file =
        std::env::temp_dir().join(format!("hex-speech-{}-{}.wav", std::process::id(), job.id));
    let (mut command, text) = match system::plan(job, &file) {
        Plan::Audio(bytes) => {
            let samples = bytes.as_deref().and_then(wav_samples);
            if bytes.is_some() && samples.is_none() {
                tracing::warn!("the speech engine wrote audio Agent-Desk can't read");
            }
            return samples;
        }
        Plan::Command { command, stdin } => (command, stdin),
    };
    command
        .stdin(if text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "could not start the speech program");
            return None;
        }
    };
    let stdin = child.stdin.take();
    {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        if generation != GENERATION.load(Ordering::Acquire) {
            crate::assistant::backends::run::kill_group(&mut child);
            return None;
        }
        *current = Some(child);
    }
    if let (Some(mut stdin), Some(text)) = (stdin, text) {
        // A stop before it's all written ends the pipe; that's fine.
        let _ = stdin.write_all(text.as_bytes());
    }
    let status = loop {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        let Some(child) = current.as_mut() else {
            // Stopped.
            let _ = std::fs::remove_file(&file);
            return None;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                current.take();
                break status;
            }
            Ok(None) => {}
            Err(_) => {
                current.take();
                return None;
            }
        }
        drop(current);
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let bytes = std::fs::read(&file);
    let _ = std::fs::remove_file(&file);
    if !status.success() {
        tracing::warn!(?status, "the speech program could not read a sentence");
        return None;
    }
    let samples = bytes.ok().and_then(|bytes| wav_samples(&bytes));
    if samples.is_none() {
        tracing::warn!("the speech program wrote audio Agent-Desk can't read");
    }
    samples
}

/// A WAV file's samples, mono at [`RATE`]: 32-bit float or 16-bit PCM at
/// any rate (the first channel of several), resampled linearly when its
/// rate differs; `None` for anything else.
pub fn wav_samples(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4)? as usize;
        let body = at + 8;
        let end = body.checked_add(size)?.min(bytes.len());
        match id {
            b"fmt " => {
                let mut tag = u16_at(body)?;
                let channels = u16_at(body + 2)?;
                let rate = u32_at(body + 4)?;
                let bits = u16_at(body + 14)?;
                // 0xFFFE is "extensible": its subformat's first two bytes
                // are the real tag (`say` writes float this way).
                if tag == 0xFFFE {
                    tag = if size >= 26 {
                        u16_at(body + 24)?
                    } else if bits == 32 {
                        3
                    } else {
                        1
                    };
                }
                format = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format?;
                if channels == 0 || rate == 0 {
                    return None;
                }
                let data = &bytes[body..end];
                let samples: Vec<f32> = match (tag, bits) {
                    (3, 32) => data
                        .chunks_exact(4 * usize::from(channels))
                        .map(|frame| f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]))
                        .collect(),
                    (1, 16) => data
                        .chunks_exact(2 * usize::from(channels))
                        .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32_768.0)
                        .collect(),
                    _ => return None,
                };
                return Some(resample(&samples, rate));
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body + size + (size & 1);
    }
    None
}

/// `samples` at `rate` as samples at [`RATE`], by linear interpolation.
fn resample(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == RATE || samples.len() < 2 {
        return samples.to_vec();
    }
    let step = f64::from(rate) / f64::from(RATE);
    let count = ((samples.len() as f64) / step).floor() as usize;
    (0..count)
        .map(|index| {
            let at = index as f64 * step;
            let low = at.floor() as usize;
            let high = (low + 1).min(samples.len() - 1);
            let weight = (at - low as f64) as f32;
            samples[low] * (1.0 - weight) + samples[high] * weight
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, channels: u16, rate: u32, samples: &[f32], padding: bool) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        bytes.extend(b"fmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(tag.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * 4 * u32::from(channels)).to_le_bytes());
        bytes.extend((4 * channels).to_le_bytes());
        bytes.extend(32_u16.to_le_bytes());
        if padding {
            bytes.extend(b"FLLR");
            bytes.extend(3_u32.to_le_bytes());
            bytes.extend([0, 0, 0, 0]);
        }
        bytes.extend(b"data");
        bytes.extend((samples.len() as u32 * 4).to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn says_float_wav_is_read() {
        let samples = [0.0, 0.25, -0.5, 1.0];
        assert_eq!(
            wav_samples(&wav(3, 1, RATE, &samples, true)).as_deref(),
            Some(&samples[..])
        );
        // Stereo: the first channel.
        assert_eq!(
            wav_samples(&wav(0xFFFE, 2, RATE, &samples, false)).as_deref(),
            Some(&[0.0, -0.5][..])
        );
        // 16-bit PCM at 32-bit sizes is nothing Agent-Desk reads.
        assert_eq!(wav_samples(&wav(1, 1, RATE, &samples, false)), None);
        assert_eq!(wav_samples(b"RIFF\0\0\0\0WAVEdata"), None);
    }

    fn pcm16(rate: u32, samples: &[i16]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        bytes.extend(b"fmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * 2).to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((samples.len() as u32 * 2).to_le_bytes());
        for sample in samples {
            bytes.extend(sample.to_le_bytes());
        }
        bytes
    }

    /// espeak-ng, pico2wave, piper, and Windows write 16-bit PCM at their
    /// own rates; it plays at the output's rate.
    #[test]
    fn pcm_wav_at_other_rates_is_resampled() {
        let samples = wav_samples(&pcm16(RATE, &[0, 16_384, -32_768])).expect("pcm");
        assert_eq!(samples, [0.0, 0.5, -1.0]);
        let slow = wav_samples(&pcm16(24_000, &[0, 16_384, 0, -16_384])).expect("resampled");
        assert_eq!(slow.len(), 8);
        assert!((slow[1] - 0.25).abs() < 1e-6, "{slow:?}");
        assert!((slow[2] - 0.5).abs() < 1e-6, "{slow:?}");
        assert_eq!(wav_samples(b"nonsense"), None);
    }
}
