//! Agent-Desk's own voice: Supertone's Supertonic 3, a small neural model that
//! runs on this computer (ONNX Runtime, CPU) and reads 31 languages,
//! German among them, far more naturally than the compact system voices.
//!
//! The person picks it in Settings like a system voice; its names are
//! [`PREFIX`] and a preset (`hex-voice:M1`). Each sentence is made on one
//! worker thread, in order, and its audio goes into the same playback as
//! the system engines' (so stop, the level, and queued sentences work the
//! same). When the model isn't installed, the answer's language isn't one
//! it reads, or making a sentence fails, the system voice reads instead.
//!
//! The weights are Open RAIL-M (use restrictions in THIRD_PARTY_NOTICES.md
//! and beside the weights); the inference is ported from Supertone's MIT
//! example.

pub mod files;
mod model;
pub mod text;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use model::{Model, Style};

/// What an Agent-Desk voice's name starts with.
pub const PREFIX: &str = "hex-voice:";

/// The presets the release ships: five women's voices, five men's.
pub const PRESETS: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "M1", "M2", "M3", "M4", "M5"];

/// The preset a bare Agent-Desk voice reads with: upstream's own default.
pub const DEFAULT_PRESET: &str = "M1";

/// How long the model stays loaded after its last sentence once the Agent-Desk
/// voice is no longer the chosen one (it holds about 400 MB). While it is
/// chosen, the model stays loaded ([`keep_warm`]).
const IDLE: Duration = Duration::from_secs(5 * 60);

/// A sentence longer than this is made clause by clause from its first
/// comma, so its first words play while the rest is made.
const SPLIT_FROM: usize = 60;
/// The shortest clause made on its own.
const MIN_CLAUSE: usize = 20;

/// The Agent-Desk voice's preset named by `voice` (`hex-voice:F2` → `F2`), if it
/// is one.
pub fn preset(voice: &str) -> Option<&'static str> {
    let rest = voice.strip_prefix(PREFIX)?;
    if rest.is_empty() {
        return Some(DEFAULT_PRESET);
    }
    PRESETS.iter().copied().find(|preset| *preset == rest)
}

/// The Agent-Desk voices as Settings lists them, once the model is installed.
pub fn voices() -> Vec<String> {
    if !files::installed() {
        return Vec::new();
    }
    PRESETS
        .iter()
        .map(|preset| format!("{PREFIX}{preset}"))
        .collect()
}

/// What makes the next sentence: the Agent-Desk voice when `chosen` names one, it
/// is installed, and it reads `language` (Supertonic's code, from
/// [`text::language_code`]); `None` for the system voice.
pub fn plan(
    chosen: Option<&str>,
    language: Option<&str>,
    installed: bool,
) -> Option<(&'static str, &'static str)> {
    let preset = preset(chosen?)?;
    let language = text::language_code(language?)?;
    installed.then_some((preset, language))
}

/// Words a long sentence without a comma may be split before.
const JOINS: &[&str] = &[
    "und", "oder", "aber", "denn", "weil", "dass", "sowie", "sodass", "and", "or", "but",
    "because", "so", "while", "et", "mais", "y", "pero", "e", "ma",
];

/// `sentence` in the pieces it is made in: its first clause on its own
/// when the sentence is long (so the first audio comes sooner), then the
/// rest. It splits after a comma, semicolon, or colon, else before a word
/// like "und".
pub fn pieces(sentence: &str) -> Vec<&str> {
    let sentence = sentence.trim();
    if sentence.chars().count() <= SPLIT_FROM {
        return vec![sentence];
    }
    let fits = |end: usize| {
        sentence[..end].trim().chars().count() >= MIN_CLAUSE
            && sentence[end..].trim().chars().count() >= MIN_CLAUSE
    };
    let joined = || {
        sentence
            .char_indices()
            .filter(|(_, c)| c.is_whitespace())
            .map(|(at, _)| at)
            .find(|&at| {
                let next = sentence[at..].trim_start();
                let word = next
                    .split(|c: char| !c.is_alphabetic())
                    .next()
                    .unwrap_or_default();
                JOINS.contains(&word) && fits(at)
            })
    };
    let split = sentence
        .char_indices()
        .filter(|(_, c)| matches!(c, ',' | ';' | ':'))
        .map(|(at, c)| at + c.len_utf8())
        .find(|&end| fits(end) && sentence[end..].starts_with(char::is_whitespace))
        .or_else(joined);
    match split {
        Some(end) => vec![sentence[..end].trim(), sentence[end..].trim()],
        None => vec![sentence],
    }
}

/// Work for the voice's thread.
enum Work {
    Sentence(Job),
    /// Load the model and this preset now, and run it once, so the first
    /// answer's first words don't wait for it.
    Warm(&'static str),
}

/// A sentence to make.
struct Job {
    id: u64,
    text: String,
    language: &'static str,
    preset: &'static str,
    generation: u64,
    rate: u32,
    deliver: Deliver,
}

/// Where a sentence's audio goes, piece by piece: samples at the playback
/// rate and whether that was the last of it, or `None` when it couldn't be
/// made (then nothing of it was delivered).
pub type Deliver = fn(u64, Option<Vec<f32>>, bool);

type Loaded = Option<(Model, HashMap<&'static str, Style>)>;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static WORKER: OnceLock<Mutex<Sender<Work>>> = OnceLock::new();
static WARM: AtomicBool = AtomicBool::new(false);
static FAILED_ONCE: AtomicBool = AtomicBool::new(false);

fn send(work: Work) -> Result<(), Work> {
    WORKER
        .get_or_init(|| Mutex::new(start()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .send(work)
        .map_err(|mpsc::SendError(work)| work)
}

/// Makes sentence `id` with `preset` in `language`, after any asked
/// before it (so a sentence is made while the one before plays); its audio
/// (at `rate`) goes to `deliver`.
pub fn speak(
    id: u64,
    text: &str,
    language: &'static str,
    preset: &'static str,
    rate: u32,
    deliver: Deliver,
) {
    let job = Job {
        id,
        text: text.to_owned(),
        language,
        preset,
        generation: GENERATION.load(Ordering::Acquire),
        rate,
        deliver,
    };
    if let Err(Work::Sentence(job)) = send(Work::Sentence(job)) {
        deliver(job.id, None, true);
    }
}

/// Keeps the model loaded and warm with `preset` while the Agent-Desk voice is
/// the one chosen (and installed); `None` lets it go after [`IDLE`].
/// Cheap to call on every settings change.
pub fn keep_warm(preset: Option<&'static str>) {
    match preset {
        Some(preset) if files::installed() => {
            WARM.store(true, Ordering::Release);
            let _ = send(Work::Warm(preset));
        }
        _ => WARM.store(false, Ordering::Release),
    }
}

/// Drops the sentences asked so far that aren't made yet.
pub fn stop() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn start() -> Sender<Work> {
    let (sender, work) = mpsc::channel::<Work>();
    let spawned = std::thread::Builder::new()
        .name("hex-neural-voice".into())
        .spawn(move || {
            let mut loaded: Loaded = None;
            loop {
                match work.recv_timeout(IDLE) {
                    Ok(Work::Sentence(job)) => sentence(&mut loaded, &job),
                    Ok(Work::Warm(preset)) => {
                        if let Err(error) = warm(&mut loaded, preset) {
                            tracing::warn!(error = %format!("{error:#}"), "Agent-Desk's voice couldn't load");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if !WARM.load(Ordering::Acquire) {
                            loaded = None;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start Agent-Desk's voice");
    }
    sender
}

/// Makes `job`'s sentence piece by piece, delivering each as it's ready.
fn sentence(loaded: &mut Loaded, job: &Job) {
    if job.generation != GENERATION.load(Ordering::Acquire) {
        return;
    }
    let pieces = pieces(&job.text);
    let mut delivered = false;
    for (index, piece) in pieces.iter().enumerate() {
        if job.generation != GENERATION.load(Ordering::Acquire) {
            return;
        }
        let last = index + 1 == pieces.len();
        let started = Instant::now();
        match make(loaded, piece, job.language, job.preset) {
            Ok((from, samples)) => {
                tracing::debug!(
                    took_ms = started.elapsed().as_millis() as u64,
                    audio_ms = (samples.len() as u64 * 1000) / u64::from(from.max(1)),
                    "Agent-Desk's voice made a piece"
                );
                (job.deliver)(job.id, Some(resample(&samples, from, job.rate)), last);
                delivered = true;
            }
            Err(error) => {
                if !FAILED_ONCE.swap(true, Ordering::AcqRel) {
                    tracing::warn!(error = %format!("{error:#}"), "Agent-Desk's voice couldn't read; the system voice reads instead");
                }
                *loaded = None;
                if delivered {
                    // Part of it played: end it there.
                    (job.deliver)(job.id, Some(Vec::new()), true);
                } else {
                    (job.deliver)(job.id, None, true);
                }
                return;
            }
        }
    }
}

/// Loads the model if it isn't, with `preset`'s style, and runs it once.
fn warm(loaded: &mut Loaded, preset: &'static str) -> color_eyre::Result<()> {
    let fresh = loaded.is_none();
    let started = Instant::now();
    let (model, styles) = ensure(loaded, preset)?;
    if fresh {
        // A session's first run allocates; do it before an answer needs it.
        model.speak(text::WARM_UP, "de", &styles[preset])?;
        tracing::info!(
            took_ms = started.elapsed().as_millis() as u64,
            "Agent-Desk's voice is loaded and warm"
        );
    }
    Ok(())
}

/// The model, loaded if it wasn't, and the styles with `preset`'s among
/// them.
fn ensure<'a>(
    loaded: &'a mut Loaded,
    preset: &'static str,
) -> color_eyre::Result<(&'a mut Model, &'a mut HashMap<&'static str, Style>)> {
    let folder = files::folder()?;
    if loaded.is_none() {
        *loaded = Some((
            Model::load(&folder.join("onnx"), threads())?,
            HashMap::new(),
        ));
    }
    let Some((model, styles)) = loaded.as_mut() else {
        unreachable!("loaded above");
    };
    if !styles.contains_key(preset) {
        let style = Style::load(&folder.join(format!("voice_styles/{preset}.json")))?;
        styles.insert(preset, style);
    }
    Ok((model, styles))
}

/// Makes `piece`: the model's sample rate and its samples.
fn make(
    loaded: &mut Loaded,
    piece: &str,
    language: &'static str,
    preset: &'static str,
) -> color_eyre::Result<(u32, Vec<f32>)> {
    let (model, styles) = ensure(loaded, preset)?;
    let samples = model.speak(piece, language, &styles[preset])?;
    Ok((model.sample_rate(), samples))
}

/// Threads each network runs on: the performance cores (Apple silicon's
/// efficiency cores slow a shared step down), else every core.
pub fn threads() -> usize {
    #[cfg(target_os = "macos")]
    {
        let mut cores: i32 = 0;
        let mut size = std::mem::size_of::<i32>();
        let name = c"hw.perflevel0.physicalcpu";
        // SAFETY: `cores` and `size` are valid for the call; the name is
        // NUL-terminated.
        let read = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut cores).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if read == 0 && cores > 0 {
            return cores as usize;
        }
    }
    std::thread::available_parallelism().map_or(4, std::num::NonZero::get)
}

/// `samples` at `from` as samples at `to`, by linear interpolation.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.len() < 2 {
        return samples.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
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

    #[test]
    fn hex_voices_are_told_by_their_prefix() {
        assert_eq!(preset("hex-voice:F2"), Some("F2"));
        assert_eq!(preset("hex-voice:"), Some(DEFAULT_PRESET));
        assert_eq!(preset("hex-voice:X9"), None);
        assert_eq!(preset("Anna"), None);
    }

    #[test]
    fn the_hex_voice_reads_only_when_installed_and_the_language_fits() {
        let hex = Some("hex-voice:M1");
        assert_eq!(plan(hex, Some("de"), true), Some(("M1", "de")));
        assert_eq!(plan(hex, Some("de-AT"), true), Some(("M1", "de")));
        // Not downloaded: the system voice.
        assert_eq!(plan(hex, Some("de"), false), None);
        // A language it doesn't read.
        assert_eq!(plan(hex, Some("zh"), true), None);
        assert_eq!(plan(hex, None, true), None);
        // A system voice chosen, or none.
        assert_eq!(plan(Some("Anna"), Some("de"), true), None);
        assert_eq!(plan(None, Some("de"), true), None);
    }

    #[test]
    fn a_long_sentence_starts_with_its_first_clause() {
        assert_eq!(pieces("Es ist halb zwei."), ["Es ist halb zwei."]);
        assert_eq!(
            pieces("Ich habe zehn Quellen gelesen, und die meisten sagen, dass der Preis sinkt."),
            [
                "Ich habe zehn Quellen gelesen,",
                "und die meisten sagen, dass der Preis sinkt."
            ]
        );
        // Too short a clause stays with the rest; so does a decimal comma.
        let short = "Ja, das habe ich erledigt, und zwar gründlich und vollständig für dich.";
        assert_eq!(
            pieces(short),
            [
                "Ja, das habe ich erledigt,",
                "und zwar gründlich und vollständig für dich."
            ]
        );
        // Without a comma: before "und", never inside a decimal number.
        assert_eq!(
            pieces("Der Kurs liegt bei 3,5 Prozent und damit deutlich über dem Wert vom Vorjahr."),
            [
                "Der Kurs liegt bei 3,5 Prozent",
                "und damit deutlich über dem Wert vom Vorjahr."
            ]
        );
        let plain = "Der günstigste Flug nach Lissabon startet am Freitag sehr früh am Morgen.";
        assert_eq!(pieces(plain), [plain]);
    }

    #[test]
    fn audio_is_resampled_to_the_playback_rate() {
        let second = vec![0.5_f32; 44_100];
        let out = resample(&second, 44_100, 48_000);
        assert!((out.len() as i64 - 48_000).abs() <= 1);
        assert!(out.iter().all(|sample| (sample - 0.5).abs() < 1e-6));
        assert_eq!(resample(&[1.0, 2.0], 48_000, 48_000), [1.0, 2.0]);
    }

    /// Reads a German sentence with the real model when
    /// `AGENT_DESK_SUPERTONIC_DIR` names its folder (the release's layout) and
    /// writes `supertonic-test.wav` beside it. Plays nothing.
    #[test]
    #[ignore = "needs the downloaded model"]
    fn a_german_sentence_is_read_by_the_real_model() {
        let Some(dir) = std::env::var_os("AGENT_DESK_SUPERTONIC_DIR").map(std::path::PathBuf::from)
        else {
            return;
        };
        assert!(files::installed_in(&dir), "every pinned file at its size");
        let started = Instant::now();
        let mut model = Model::load(&dir.join("onnx"), threads()).unwrap();
        let loaded = started.elapsed();
        let style = Style::load(&dir.join("voice_styles/M1.json")).unwrap();
        let sentence = "Es ist gerade halb zwei, und in Tokio regnet es leicht.";
        for round in 0..2 {
            let started = Instant::now();
            let samples = model.speak(sentence, "de", &style).unwrap();
            let took = started.elapsed();
            let seconds = samples.len() as f64 / f64::from(model.sample_rate());
            eprintln!(
                "round {round}: load {loaded:?}, made {seconds:.2} s of audio in {took:?} (RTF {:.3})",
                took.as_secs_f64() / seconds
            );
            assert!(seconds > 1.0 && seconds < 10.0);
            assert!(samples.iter().all(|sample| sample.is_finite()));
            if round == 1 {
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: model.sample_rate(),
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut writer =
                    hound::WavWriter::create(dir.join("supertonic-test.wav"), spec).unwrap();
                for sample in samples {
                    writer
                        .write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                        .unwrap();
                }
                writer.finalize().unwrap();
            }
        }
    }

    /// How long a typical answer's first sentence takes by steps and
    /// threads, and how far fewer steps stray from many (the frame
    /// loudness envelope's correlation with 16 steps'). Plays nothing.
    #[test]
    #[ignore = "needs the downloaded model; a benchmark"]
    fn steps_and_threads_benchmark() {
        let Some(dir) = std::env::var_os("AGENT_DESK_SUPERTONIC_DIR").map(std::path::PathBuf::from)
        else {
            return;
        };
        let style = Style::load(&dir.join("voice_styles/M1.json")).unwrap();
        let sentences = [
            "Es ist gerade halb zwei.",
            "Ich habe zehn Quellen gelesen, und die meisten sagen, dass der Preis im Frühjahr sinkt.",
        ];
        let envelope = |samples: &[f32]| -> Vec<f32> {
            samples
                .chunks(2_205)
                .map(|chunk| (chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32).sqrt())
                .collect()
        };
        let correlation = |a: &[f32], b: &[f32]| -> f32 {
            let n = a.len().min(b.len());
            let (a, b) = (&a[..n], &b[..n]);
            let mean = |v: &[f32]| v.iter().sum::<f32>() / n as f32;
            let (ma, mb) = (mean(a), mean(b));
            let cov: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
            let va: f32 = a.iter().map(|x| (x - ma) * (x - ma)).sum();
            let vb: f32 = b.iter().map(|y| (y - mb) * (y - mb)).sum();
            cov / (va.sqrt() * vb.sqrt()).max(1e-9)
        };
        for threads in [threads(), 12, 18] {
            let mut model = Model::load(&dir.join("onnx"), threads).unwrap();
            // Warm-up: the first run of a session is slower.
            model.speak_with(sentences[0], "de", &style, 2).unwrap();
            for sentence in sentences {
                let reference = envelope(&model.speak_with(sentence, "de", &style, 16).unwrap());
                for steps in [0, 2, 3, 4, 5, 6, 8] {
                    let started = Instant::now();
                    let samples = model.speak_with(sentence, "de", &style, steps).unwrap();
                    let took = started.elapsed();
                    let seconds = samples.len() as f64 / f64::from(model.sample_rate());
                    eprintln!(
                        "threads {threads:2} steps {steps}: {:>6.1} ms for {seconds:.2} s (RTF {:.3}), envelope r={:.3}  [{}…]",
                        took.as_secs_f64() * 1000.0,
                        took.as_secs_f64() / seconds,
                        correlation(&envelope(&samples), &reference),
                        &sentence[..12],
                    );
                    if threads == super::threads() && sentence == sentences[1] {
                        let spec = hound::WavSpec {
                            channels: 1,
                            sample_rate: model.sample_rate(),
                            bits_per_sample: 16,
                            sample_format: hound::SampleFormat::Int,
                        };
                        let mut writer =
                            hound::WavWriter::create(dir.join(format!("steps-{steps}.wav")), spec)
                                .unwrap();
                        for sample in &samples {
                            writer
                                .write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                                .unwrap();
                        }
                        writer.finalize().unwrap();
                    }
                }
            }
        }
    }

    /// Time to first audio as the worker makes it: the model loaded and
    /// warmed ([`warm`]), then each answer's first piece at [`model::STEPS`].
    #[test]
    #[ignore = "needs the downloaded model; a benchmark"]
    fn time_to_first_audio() {
        let Some(dir) = std::env::var_os("AGENT_DESK_SUPERTONIC_DIR").map(std::path::PathBuf::from)
        else {
            return;
        };
        let started = Instant::now();
        let mut model = Model::load(&dir.join("onnx"), threads()).unwrap();
        let style = Style::load(&dir.join("voice_styles/M1.json")).unwrap();
        model.speak(text::WARM_UP, "de", &style).unwrap();
        eprintln!(
            "load + warm-up: {:?} ({} threads)",
            started.elapsed(),
            threads()
        );
        let answers = [
            "Es ist gerade halb zwei.",
            "Ich habe zehn Quellen gelesen, und die meisten sagen, dass der Preis im Frühjahr sinkt. Die Details stehen im Bericht.",
            "Der günstigste Flug nach Lissabon kostet 89 Euro und startet am Freitag um sieben Uhr.",
            "Erledigt. Ich habe die drei Dateien in deinen Downloads-Ordner gelegt.",
        ];
        for answer in answers {
            let first = answer.split_inclusive(". ").next().unwrap_or(answer);
            let piece = pieces(first)[0];
            let started = Instant::now();
            let samples = model.speak(piece, "de", &style).unwrap();
            let ttfa = started.elapsed();
            let seconds = samples.len() as f64 / f64::from(model.sample_rate());
            let started = Instant::now();
            let mut total_audio = 0.0;
            for sentence in answer.split_inclusive(". ") {
                for piece in pieces(sentence) {
                    let samples = model.speak(piece, "de", &style).unwrap();
                    total_audio += samples.len() as f64 / f64::from(model.sample_rate());
                }
            }
            let all = started.elapsed();
            eprintln!(
                "TTFA {:>5.0} ms (first piece {seconds:.2} s: {piece:?}); whole answer {total_audio:.2} s made in {:.0} ms (RTF {:.3})",
                ttfa.as_secs_f64() * 1000.0,
                all.as_secs_f64() * 1000.0,
                all.as_secs_f64() / total_audio,
            );
        }
    }
}
