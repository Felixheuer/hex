//! The fallback engine: `/usr/bin/say` writes each sentence to a WAV file,
//! which plays through the same output as the speech engine's audio, so
//! stopping, queueing, and the level work the same.

use std::io::Write;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use super::playback::RATE;

const SAY: &str = "/usr/bin/say";
/// Words a minute: a little faster than `say`'s 175, as the speech engine
/// reads.
const WORDS_PER_MINUTE: &str = "190";

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

/// Has `say` write `job` to a file and reads it back.
fn write(job: &Job, generation: u64) -> Option<Vec<f32>> {
    let file =
        std::env::temp_dir().join(format!("hex-speech-{}-{}.wav", std::process::id(), job.id));
    let mut command = crate::desktop::background_process::command(SAY);
    if let Some(voice) = &job.voice {
        command.args(["-v", voice]);
    }
    command
        .args(["-r", WORDS_PER_MINUTE])
        .args(["--file-format=WAVE", &format!("--data-format=LEF32@{RATE}")])
        .arg("-o")
        .arg(&file)
        // The text goes in on stdin, so one that starts with "-" is never
        // an option.
        .args(["-f", "-"])
        .stdin(Stdio::piped())
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
            tracing::warn!(%error, "could not start say");
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
    if let Some(mut stdin) = stdin {
        // A stop before it's all written ends the pipe; that's fine.
        let _ = stdin.write_all(job.text.as_bytes());
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
        tracing::warn!(?status, "say could not read a sentence");
        return None;
    }
    let samples = bytes.ok().and_then(|bytes| wav_samples(&bytes));
    if samples.is_none() {
        tracing::warn!("say wrote audio Hex can't read");
    }
    samples
}

/// A mono 32-bit float WAV's samples at [`RATE`] (the first channel of
/// several); `None` for anything else.
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
                let tag = u16_at(body)?;
                let channels = u16_at(body + 2)?;
                let rate = u32_at(body + 4)?;
                let bits = u16_at(body + 14)?;
                // 3 is IEEE float; 0xFFFE is "extensible", which `say` uses
                // for float too.
                format = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format?;
                if !matches!(tag, 3 | 0xFFFE) || bits != 32 || rate != RATE || channels == 0 {
                    return None;
                }
                let frame = 4 * usize::from(channels);
                return Some(
                    bytes[body..end]
                        .chunks_exact(frame)
                        .map(|frame| f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]))
                        .collect(),
                );
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body + size + (size & 1);
    }
    None
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
        assert_eq!(wav_samples(&wav(1, 1, RATE, &samples, false)), None);
        assert_eq!(wav_samples(&wav(3, 1, 22_050, &samples, false)), None);
        assert_eq!(wav_samples(b"RIFF\0\0\0\0WAVEdata"), None);
        assert_eq!(wav_samples(b"nonsense"), None);
    }
}
