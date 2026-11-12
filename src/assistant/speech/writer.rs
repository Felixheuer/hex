//! The engine that writes each sentence whole before any of it plays: `say`
//! on macOS (where the speech engine can't answer), Windows' own voices,
//! and Piper or espeak-ng on Linux. Its audio plays through the same output
//! as the speech engine's, so stopping, queueing, and the level work the
//! same. One worker thread writes the sentences in turn; a stop drops the
//! queue and ends the one being written.

use std::process::Child;
#[cfg(not(target_os = "windows"))]
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

use super::voices::Voice;

#[cfg(target_os = "linux")]
use super::linux as platform;
#[cfg(target_os = "macos")]
use super::say as platform;
#[cfg(target_os = "windows")]
use super::windows as platform;

/// A sentence to write.
pub struct Job {
    pub id: u64,
    pub text: String,
    /// The voice asked for, as the caller named it.
    pub chosen: Option<String>,
}

/// Sentence audio (mono, at the output's rate), or `None` when nothing
/// could be made of it.
pub type Deliver = fn(u64, Option<Vec<f32>>);

struct Worker {
    jobs: Sender<(u64, Job)>,
}

/// The program writing now, so a stop can end it.
static CURRENT: Mutex<Option<Child>> = Mutex::new(None);
/// Bumped by a stop: queued sentences from before it are skipped.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn worker(deliver: Deliver) -> Option<&'static Worker> {
    static WORKER: OnceLock<Option<Worker>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (jobs, receiver) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("hex-speech-writer".into())
                .spawn(move || work(&receiver, deliver))
                .map_err(|error| tracing::warn!(%error, "could not start reading aloud"))
                .ok()?;
            Some(Worker { jobs })
        })
        .as_ref()
}

/// Asks for `job`'s audio, after the sentences before it.
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
    platform::stopped();
}

/// The installed voices this engine reads with.
pub fn voices() -> Vec<Voice> {
    platform::voices()
}

fn work(jobs: &Receiver<(u64, Job)>, deliver: Deliver) {
    platform::prepare_thread();
    while let Ok((generation, job)) = jobs.recv() {
        if generation != GENERATION.load(Ordering::Acquire) {
            continue;
        }
        let voices = platform::voices();
        let voice = super::pick(&job.text, job.chosen.as_deref(), &voices);
        tracing::debug!(
            voice = voice.map_or("system", |voice| voice.name.as_str()),
            "reading aloud"
        );
        let audio = platform::write(&job.text, voice, generation);
        if generation == GENERATION.load(Ordering::Acquire) {
            deliver(job.id, audio);
        }
    }
}

/// Whether a stop came since `generation`.
pub fn stopped_since(generation: u64) -> bool {
    generation != GENERATION.load(Ordering::Acquire)
}

/// Runs `command` (its own process group) with `text` on its stdin until
/// it ends; a stop ends it early. Returns how it ended and what it wrote to
/// stdout, or `None` when stopped or it couldn't start.
#[cfg(not(target_os = "windows"))]
pub fn run(mut command: Command, text: &str, generation: u64) -> Option<(ExitStatus, Vec<u8>)> {
    use std::io::{Read, Write};
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, program = ?command.get_program(), "could not start reading aloud");
            return None;
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        if stopped_since(generation) {
            crate::assistant::backends::run::kill_group(&mut child);
            return None;
        }
        *current = Some(child);
    }
    // Written and read on threads of their own, so a full pipe never
    // stalls the other side.
    let text = text.to_owned();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            // A stop before it's all written ends the pipe; that's fine.
            let _ = stdin.write_all(text.as_bytes());
        }
    });
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut bytes);
        }
        bytes
    });
    let status = loop {
        let mut current = CURRENT.lock().unwrap_or_else(|error| error.into_inner());
        let Some(child) = current.as_mut() else {
            // Stopped.
            let _ = writer.join();
            let _ = reader.join();
            return None;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                current.take();
                break status;
            }
            Ok(None) => {}
            Err(_) => {
                if let Some(mut child) = current.take() {
                    crate::assistant::backends::run::kill_group(&mut child);
                }
                return None;
            }
        }
        drop(current);
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let _ = writer.join();
    let bytes = reader.join().unwrap_or_default();
    Some((status, bytes))
}

/// A file in the temp folder for one sentence's audio.
#[cfg(not(target_os = "windows"))]
pub fn temp_file(extension: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "hex-speech-{}-{}.{extension}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Reads and removes the audio file a program wrote, after it ended with
/// `status`.
#[cfg(not(target_os = "windows"))]
pub fn take_file(file: &std::path::Path, status: Option<ExitStatus>) -> Option<Vec<f32>> {
    let bytes = std::fs::read(file);
    let _ = std::fs::remove_file(file);
    let status = status?;
    if !status.success() {
        tracing::warn!(?status, "the voice could not read a sentence");
        return None;
    }
    let samples = bytes.ok().and_then(|bytes| super::wav::samples(&bytes));
    if samples.is_none() {
        tracing::warn!("the voice wrote audio Hex can't read");
    }
    samples
}
