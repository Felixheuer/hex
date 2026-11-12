//! Reading answers aloud on this computer, with the best voice installed
//! for the answer's language.
//!
//! macOS's speech engine (AVSpeechSynthesizer, through the Swift bridge in
//! `src/speech_output.swift`) writes each sentence to audio, which Hex
//! plays itself ([`output`]): so sentences follow each other without the
//! engine's long pauses, a stop is heard at once, and [`level`] measures
//! what plays. Where the engine can't answer (nothing serves the main
//! thread, as in tests), `/usr/bin/say` writes the audio instead ([`say`]).
//!
//! The person's chosen voice wins. Otherwise the answer's language picks:
//! their Personal Voice when Hex may use it, then Premium, then Enhanced
//! voices, then the default voice of that language.
//!
//! OWNER: speech output.

#[cfg(target_os = "macos")]
mod bridge;
mod language;
pub mod neural;
mod output;
pub mod own_echo;
mod playback;
mod say;
mod voices;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use language::{language_of, system_language};
use output::Shared;
use playback::{Engine, Request};
pub use voices::{Quality, Voice, VoiceHint};

/// AVSpeechUtterance's rate (0.5 is its normal): a little brisker, as people
/// talk to an assistant.
const ENGINE_RATE: f32 = 0.53;
/// How long the main queue may take to answer before `say` reads instead.
const MAIN_TIMEOUT_MS: i32 = 250;
/// How long the speech engine is left alone after it failed to answer.
const ENGINE_REST: Duration = Duration::from_secs(60);
/// How long an answered main queue counts as answering.
const MAIN_FRESH: Duration = Duration::from_secs(5);
/// How long a voice list is kept (voices can be downloaded meanwhile).
const VOICES_FRESH: Duration = Duration::from_secs(60);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The queue and the output, started with the first sentence.
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

/// The output, once something was read (a stop before then has nothing to
/// stop).
fn started() -> Option<&'static Arc<Shared>> {
    SHARED.get()
}

fn shared() -> &'static Arc<Shared> {
    SHARED.get_or_init(|| {
        let shared = Arc::new(Shared::default());
        let thread = shared.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("hex-speech-output".into())
            .spawn(move || output::run(thread.clone(), || tick(&thread)))
        {
            tracing::warn!(%error, "could not start the speech output");
        }
        shared
    })
}

/// Hands sentences the speech engine never answered to `say`; whether
/// anything is queued.
fn tick(shared: &Shared) -> bool {
    let moved = shared.playback().expire(Instant::now());
    if !moved.is_empty() {
        tracing::warn!(
            sentences = moved.len(),
            "the speech engine didn't answer; reading with say"
        );
        rest_engine();
        for request in moved {
            ask(Engine::Say, &request);
        }
    }
    shared.playback().busy()
}

/// Starts reading `text` aloud with `voice`, stopping anything already
/// being read. `None` picks by the text's own language
/// ([`automatic_voice`]). Returns at once.
pub fn speak(text: &str, voice: Option<&str>) {
    stop();
    queue(text, voice);
}

/// Reads `text` aloud with `voice` after what is being read, without a
/// pause between them: for an answer streamed sentence by sentence.
/// Returns at once.
#[allow(
    dead_code,
    reason = "for the brain's streamed answers (service::voice)"
)]
pub fn queue(text: &str, voice: Option<&str>) {
    let text = speakable(text);
    if text.is_empty() {
        return;
    }
    own_echo::note(&text);
    let request = Request {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        text,
        chosen: voice.map(str::to_owned),
    };
    let engine = if neural_plan(&request).is_some() {
        Engine::Neural
    } else {
        engine()
    };
    let shared = shared();
    shared
        .playback()
        .add(request.clone(), engine, Instant::now());
    shared.asked();
    ask(engine, &request);
}

/// Asks `engine` for `request`'s audio.
fn ask(engine: Engine, request: &Request) {
    match engine {
        #[cfg(target_os = "macos")]
        Engine::Bridge => {
            let voices = engine_voices();
            let voice = pick(request, &voices);
            tracing::debug!(
                voice = voice.map_or("system", |voice| voice.name.as_str()),
                "reading aloud"
            );
            bridge::speak(
                request.id,
                &request.text,
                voice.map(|voice| voice.id.as_str()),
                ENGINE_RATE,
            );
        }
        #[cfg(not(target_os = "macos"))]
        Engine::Bridge => {}
        Engine::Neural => match neural_plan(request) {
            Some((preset, language)) => {
                tracing::debug!(preset, language, "reading aloud with Hex's voice");
                neural::speak(
                    request.id,
                    &request.text,
                    language,
                    preset,
                    playback::RATE,
                    from_neural,
                );
            }
            None => from_neural(request.id, None, true),
        },
        Engine::Say => {
            let voices = say_voices();
            let voice = pick(request, &voices).map(|voice| voice.id.clone());
            tracing::debug!(
                voice = voice.as_deref().unwrap_or("system"),
                "reading aloud with say"
            );
            say::speak(
                say::Job {
                    id: request.id,
                    text: request.text.clone(),
                    voice,
                },
                from_say,
            );
        }
    }
}

/// The voice for `request`: the one asked for, if installed, else the best
/// for its language.
fn pick<'a>(request: &Request, voices: &'a [Voice]) -> Option<&'a Voice> {
    let language = language_of(&request.text);
    request
        .chosen
        .as_deref()
        .and_then(|chosen| voices::resolve(chosen, language, voices))
        .or_else(|| {
            let system = language::system_language();
            voices::best_voice(
                language.unwrap_or(&system),
                &language::system_locale(),
                voices,
            )
        })
}

/// Stops reading aloud at once and forgets what was queued.
pub fn stop() {
    let Some(shared) = started() else {
        return;
    };
    shared.stop();
    neural::stop();
    #[cfg(target_os = "macos")]
    if ENGINE_STARTED.load(Ordering::Acquire) {
        bridge::stop();
    }
    say::stop();
}

/// Whether something is being read now, or waits its turn.
pub fn speaking() -> bool {
    started().is_some_and(|shared| shared.playback().busy())
}

/// When the audio playing now started; `None` while nothing plays. Stays
/// put across sentences queued back to back.
#[allow(dead_code, reason = "for echo handling in conversation mode")]
pub fn speaking_since() -> Option<Instant> {
    started().and_then(|shared| shared.playback().since())
}

/// How loud the answer being read sounds now: the linear RMS (0 to 1) of
/// the last 30 ms the output played, or `None` while nothing plays.
/// Barge-in on speakers needs it (`recognition::conversation`); cheap
/// enough for every frame.
pub fn level() -> Option<f32> {
    let shared = started()?;
    shared.playback().since()?;
    Some(shared.level())
}

/// Which engine reads the next sentence.
fn engine() -> Engine {
    #[cfg(target_os = "macos")]
    if engine_answers() {
        return Engine::Bridge;
    }
    Engine::Say
}

#[cfg(target_os = "macos")]
static ENGINE_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// When the speech engine may be tried again, and when its main queue
/// last answered.
static ENGINE_STATE: Mutex<(Option<Instant>, Option<Instant>)> = Mutex::new((None, None));

fn rest_engine() {
    ENGINE_STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .0 = Some(Instant::now() + ENGINE_REST);
}

/// Whether the speech engine can read now: it lists voices, and the main
/// queue it calls back on answers.
#[cfg(target_os = "macos")]
fn engine_answers() -> bool {
    let now = Instant::now();
    {
        let state = ENGINE_STATE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.0.is_some_and(|until| now < until) {
            return false;
        }
        if state
            .1
            .is_some_and(|at| now.duration_since(at) < MAIN_FRESH)
        {
            return true;
        }
    }
    if engine_voices().is_empty() {
        return false;
    }
    if !ENGINE_STARTED.swap(true, Ordering::AcqRel) {
        bridge::start(from_bridge);
    }
    match bridge::main_state(MAIN_TIMEOUT_MS) {
        bridge::Main::Answers => {
            ENGINE_STATE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .1 = Some(now);
            true
        }
        bridge::Main::Pumped => true,
        bridge::Main::Silent => {
            tracing::debug!("the main queue didn't answer; reading with say");
            rest_engine();
            false
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" fn from_bridge(id: u64, samples: *const f32, count: usize, done: bool) {
    let samples = if samples.is_null() || count == 0 {
        &[][..]
    } else {
        // SAFETY: The bridge passes `count` valid samples for this call.
        unsafe { std::slice::from_raw_parts(samples, count) }
    };
    if let Some(shared) = started() {
        shared
            .playback()
            .feed(Engine::Bridge, id, samples, done, Instant::now());
    }
}

/// Hex's voice for `request`, when it was chosen, is installed, and reads
/// the sentence's language (the system's when the sentence doesn't tell):
/// its preset and language.
fn neural_plan(request: &Request) -> Option<(&'static str, &'static str)> {
    let chosen = request.chosen.as_deref()?;
    neural::preset(chosen)?;
    let system = language::system_language();
    let language = language_of(&request.text).unwrap_or(&system);
    neural::plan(Some(chosen), Some(language), neural::files::installed())
}

/// Hex's voice made a piece of sentence `id` (`done` with its last), or
/// couldn't make it: then the system engine reads it.
fn from_neural(id: u64, audio: Option<Vec<f32>>, done: bool) {
    let Some(shared) = started() else {
        return;
    };
    let now = Instant::now();
    if let Some(audio) = audio {
        shared
            .playback()
            .feed(Engine::Neural, id, &audio, done, now);
        return;
    }
    let fallback = engine();
    let request = shared
        .playback()
        .reassign(id, Engine::Neural, fallback, now);
    if let Some(request) = request {
        ask(fallback, &request);
    }
}

fn from_say(id: u64, audio: Option<Vec<f32>>) {
    if let Some(shared) = started() {
        shared.playback().feed(
            Engine::Say,
            id,
            &audio.unwrap_or_default(),
            true,
            Instant::now(),
        );
    }
}

/// The installed voices' names, for Settings: Hex's own first, once
/// downloaded, then the system's.
pub fn voices() -> Vec<String> {
    let mut names = neural::voices();
    names.extend(voices_with_languages().into_iter().map(|voice| voice.name));
    names
}

/// The installed voices with their languages and quality: the speech
/// engine's, or `say`'s where the engine lists none.
pub fn voices_with_languages() -> Vec<Voice> {
    #[cfg(target_os = "macos")]
    {
        let voices = engine_voices();
        if !voices.is_empty() {
            return voices;
        }
    }
    say_voices()
}

/// AVSpeechSynthesizer's voices, named for Settings.
#[cfg(target_os = "macos")]
fn engine_voices() -> Vec<Voice> {
    static VOICES: Mutex<Option<(Instant, Vec<Voice>)>> = Mutex::new(None);
    let mut cached = VOICES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((at, voices)) = cached.as_ref()
        && at.elapsed() < VOICES_FRESH
    {
        return voices.clone();
    }
    let voices = voices::named(bridge::voices());
    *cached = Some((Instant::now(), voices.clone()));
    voices
}

/// The speech program's voices (`say -v '?'` on macOS; Piper, Pico, and
/// eSpeak on Linux; the Windows voices), read again after a minute, since
/// voices can be installed meanwhile.
fn say_voices() -> Vec<Voice> {
    static VOICES: Mutex<Option<(Instant, Vec<Voice>)>> = Mutex::new(None);
    let mut cached = VOICES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((at, voices)) = cached.as_ref()
        && at.elapsed() < VOICES_FRESH
    {
        return voices.clone();
    }
    let voices = say::voices();
    *cached = Some((Instant::now(), voices.clone()));
    voices
}

/// The voice [`speak`] reads `text` with when none is chosen: the best
/// installed voice of the text's language (the system's language when the
/// text doesn't tell), or `None` when there is none and the system voice
/// reads.
pub fn automatic_voice(text: &str) -> Option<String> {
    let voices = voices_with_languages();
    let system = language::system_language();
    voices::best_voice(
        language_of(text).unwrap_or(&system),
        &language::system_locale(),
        &voices,
    )
    .map(|voice| voice.name.clone())
}

/// The voice to download for `language` (`de`, `en-US`) when every
/// installed one of it sounds worse: its name in System Settings, and about
/// how big it is. `None` when the best is installed.
#[allow(dead_code, reason = "for the assistant's setup and Settings")]
pub fn better_voice_available(language: &str) -> Option<VoiceHint> {
    voices::better_voice_for(language, &voices_with_languages())
}

/// Whether this computer has a page where voices are installed
/// ([`open_voice_settings`]): macOS and Windows.
pub const fn has_voice_settings() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Opens where voices are installed: System Settings › Accessibility ›
/// Spoken Content on macOS (System Voice › Manage Voices…), Settings › Time
/// & language › Speech on Windows. Linux has no such page; its voices are
/// packages (`piper`, `pico2wave`, `espeak-ng`).
#[allow(dead_code, reason = "for the assistant's setup and Settings")]
pub fn open_voice_settings() {
    #[cfg(target_os = "windows")]
    {
        let opened = crate::desktop::background_process::command("explorer.exe")
            .arg("ms-settings:speech")
            .spawn();
        if let Err(error) = opened {
            tracing::warn!(%error, "could not open the speech settings");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let opened = [
            "x-apple.systempreferences:com.apple.Accessibility-Settings.extension?SpokenContent",
            "x-apple.systempreferences:com.apple.preference.universalaccess?SpeakableItems",
        ]
        .into_iter()
        .any(|url| {
            crate::desktop::background_process::command("/usr/bin/open")
                .arg(url)
                .status()
                .is_ok_and(|status| status.success())
        });
        if !opened {
            tracing::warn!("could not open the Spoken Content settings");
        }
    }
}

/// `text` as it should be heard: Markdown's marks and code fences dropped.
fn speakable(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .map(|line| {
            line.trim_start_matches(['#', '>', ' '])
                .replace("**", "")
                .replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_not_read_out() {
        assert_eq!(
            speakable("## Result\n**Done**, see `notes.txt`.\n```sh\nls\n```"),
            "Result\nDone, see notes.txt.\nls"
        );
    }

    #[test]
    fn a_chosen_voice_wins_and_else_the_language_picks() {
        let voices = voices::named(vec![
            voices::Listed {
                id: "anna".into(),
                name: "Anna".into(),
                locale: "de-DE".into(),
                quality: Quality::Default,
            },
            voices::Listed {
                id: "petra".into(),
                name: "Petra".into(),
                locale: "de-DE".into(),
                quality: Quality::Premium,
            },
            voices::Listed {
                id: "ava".into(),
                name: "Ava".into(),
                locale: "en-US".into(),
                quality: Quality::Enhanced,
            },
        ]);
        let request = |text: &str, chosen: Option<&str>| Request {
            id: 1,
            text: text.into(),
            chosen: chosen.map(str::to_owned),
        };
        let picked = |request| pick(&request, &voices).map(|voice| voice.id.clone());
        assert_eq!(
            picked(request("Die Datei ist hier.", None)).as_deref(),
            Some("petra")
        );
        assert_eq!(
            picked(request("The file is here.", None)).as_deref(),
            Some("ava")
        );
        assert_eq!(
            picked(request("The file is here.", Some("Anna"))).as_deref(),
            Some("anna")
        );
        // A voice no longer installed: the language picks.
        assert_eq!(
            picked(request("Die Datei ist hier.", Some("Gone"))).as_deref(),
            Some("petra")
        );
    }

    /// Reads one short German sentence through the real output, then cuts
    /// a second one mid-way. Plays sound: run alone, by name.
    #[test]
    #[ignore = "plays audio"]
    fn live_speak_then_stop_mid_sentence() {
        let installed = voices_with_languages();
        let best = automatic_voice("Hallo, ich bin dein Assistent.");
        eprintln!(
            "{} voices; German: {best:?}; better: {:?}",
            installed.len(),
            better_voice_available("de")
        );
        let started = Instant::now();
        speak("Hallo, ich bin dein Assistent.", None);
        let engine = shared().playback().busy();
        assert!(engine && speaking());
        while speaking_since().is_none() {
            assert!(started.elapsed() < Duration::from_secs(10), "never played");
            std::thread::sleep(Duration::from_millis(5));
        }
        let first_audio = started.elapsed();
        let mut loudest: f32 = 0.0;
        while speaking() {
            loudest = loudest.max(level().unwrap_or_default());
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!(
            "first audio after {} ms, read in {} ms, loudest level {loudest:.3}",
            first_audio.as_millis(),
            started.elapsed().as_millis()
        );
        assert!(loudest > 0.01, "nothing audible was measured");
        assert_eq!(level(), None);

        speak(
            "Dieser zweite Satz ist absichtlich ziemlich lang, damit er mitten im Lesen abgebrochen werden kann.",
            None,
        );
        while level().is_none_or(|level| level < 0.01) {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(600));
        let stopping = Instant::now();
        stop();
        let stopped = stopping.elapsed();
        assert!(!speaking());
        assert_eq!(level(), None);
        assert_eq!(speaking_since(), None);
        eprintln!("stop() returned in {} µs", stopped.as_micros());
        assert!(stopped < Duration::from_millis(50));
        // What the device still had fades within its buffer.
        std::thread::sleep(Duration::from_millis(50));
        let shared = super::started().unwrap();
        assert!(shared.level() < 0.001, "still sounding: {}", shared.level());
    }
}
