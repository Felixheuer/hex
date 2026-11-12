//! Linux's voices, best first: Piper's natural voices where Piper and a
//! model for the language are installed, else espeak-ng's, each writing a
//! sentence to WAV that Hex plays ([`super::writer`]). With neither,
//! speech-dispatcher's `spd-say` reads aloud itself: then nothing plays
//! through Hex, so the level reads nothing and talking over an answer can't
//! stop it.
//!
//! Piper models (`*.onnx` with their `.onnx.json`) are looked for in
//! `$PIPER_VOICES`, `~/.local/share/piper/voices` (and `…/piper`), and
//! `/usr/share/piper-voices` / `/usr/share/piper/voices`. What's installed
//! is found on a thread of its own and kept for a minute; only the very
//! first look waits for it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::catalog::{self, ESPEAK, PIPER};
use super::voices::{self, Voice};

/// How long what's installed is kept (voices can be added meanwhile).
const FRESH: Duration = Duration::from_secs(60);
/// How long the first look waits for what's installed.
const FIRST_LOOK: Duration = Duration::from_secs(2);
/// espeak-ng's words a minute: a little faster than its 175.
const ESPEAK_RATE: &str = "185";
/// How deep model folders are searched.
const MODEL_DEPTH: usize = 3;

/// The engines and voices found.
#[derive(Default)]
struct Installed {
    piper: Option<PathBuf>,
    espeak: Option<PathBuf>,
    spd_say: Option<PathBuf>,
    voices: Vec<Voice>,
}

#[derive(Default)]
struct Cache {
    found: Option<(Instant, Arc<Installed>)>,
    looking: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    found: None,
    looking: false,
});
static FOUND: Condvar = Condvar::new();
/// `spd-say` read the last sentence, so a stop must cancel it too.
static SPEECH_DISPATCHER: AtomicBool = AtomicBool::new(false);

/// What's installed: as last found (looked for again in the background once
/// it's a minute old), waiting only the first time.
fn installed() -> Arc<Installed> {
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let stale = cache
        .found
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() >= FRESH);
    if stale && !cache.looking {
        cache.looking = true;
        let spawned = std::thread::Builder::new()
            .name("hex-speech-voices".into())
            .spawn(|| {
                let found = Arc::new(look());
                let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
                cache.found = Some((Instant::now(), found));
                cache.looking = false;
                FOUND.notify_all();
            });
        if spawned.is_err() {
            cache.looking = false;
        }
    }
    if cache.found.is_none() && cache.looking {
        cache = FOUND
            .wait_timeout_while(cache, FIRST_LOOK, |cache| cache.found.is_none())
            .unwrap_or_else(|error| error.into_inner())
            .0;
    }
    cache
        .found
        .as_ref()
        .map(|(_, found)| found.clone())
        .unwrap_or_default()
}

fn look() -> Installed {
    let piper =
        executable("piper-tts").or_else(|| executable("piper").filter(|path| is_piper_tts(path)));
    let espeak = executable("espeak-ng").or_else(|| executable("espeak"));
    let mut listed = Vec::new();
    if piper.is_some() {
        let mut models = Vec::new();
        for folder in model_folders() {
            find_models(&folder, MODEL_DEPTH, &mut models);
        }
        models.sort();
        models.dedup();
        listed.extend(
            models
                .iter()
                .filter_map(|model| catalog::piper_voice(model)),
        );
    }
    if let Some(espeak) = &espeak {
        match crate::desktop::background_process::command(espeak)
            .arg("--voices")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(output) => listed.extend(catalog::espeak_voices(&String::from_utf8_lossy(
                &output.stdout,
            ))),
            Err(error) => tracing::warn!(%error, "could not list espeak-ng's voices"),
        }
    }
    Installed {
        piper,
        espeak,
        spd_say: executable("spd-say"),
        voices: voices::named(listed),
    }
}

/// `name` on `PATH`.
fn executable(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// Whether a `piper` on `PATH` is the speech engine, not libratbag's mouse
/// settings app of the same name (a GTK program).
fn is_piper_tts(path: &Path) -> bool {
    let mut head = vec![0; 4096];
    let read = std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read(&mut file, &mut head))
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..read]);
    !head.contains("ratbag") && !head.contains("gi.require_version")
}

fn model_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("PIPER_VOICES")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")));
    if let Some(data) = data {
        folders.push(data.join("piper/voices"));
        folders.push(data.join("piper"));
    }
    folders.extend(
        [
            "/usr/share/piper-voices",
            "/usr/share/piper/voices",
            "/usr/local/share/piper/voices",
        ]
        .map(PathBuf::from),
    );
    folders
}

/// The `.onnx` models under `folder` that have their `.onnx.json` beside
/// them.
fn find_models(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                find_models(&path, depth - 1, found);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "onnx")
            && path.with_extension("onnx.json").is_file()
        {
            found.push(path);
        }
    }
}

/// Nothing to set up on the writer's thread.
pub fn prepare_thread() {}

/// Cancels what speech-dispatcher still reads: ending `spd-say` doesn't.
pub fn stopped() {
    if SPEECH_DISPATCHER.swap(false, Ordering::AcqRel)
        && let Some(spd_say) = installed().spd_say.clone()
    {
        // On a thread of its own: a stop returns at once.
        std::thread::spawn(move || {
            let _ = crate::desktop::background_process::command(spd_say)
                .arg("--cancel")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

/// The installed Piper and espeak-ng voices.
pub fn voices() -> Vec<Voice> {
    installed().voices.clone()
}

/// `text` read by `voice` as samples; `None` after `spd-say` read it
/// itself, or when nothing could.
pub fn write(text: &str, voice: Option<&Voice>, generation: u64) -> Option<Vec<f32>> {
    let installed = installed();
    let id = voice.map(|voice| voice.id.as_str()).unwrap_or_default();
    if let (Some(model), Some(piper)) = (id.strip_prefix(PIPER), &installed.piper) {
        return piper_write(piper, Path::new(model), text, generation);
    }
    let language = super::language_of(text)
        .map(str::to_owned)
        .unwrap_or_else(super::language::system_language);
    if let Some(espeak) = &installed.espeak {
        let espeak_voice = id.strip_prefix(ESPEAK).unwrap_or(&language);
        let mut command = crate::desktop::background_process::command(espeak);
        command.args(["--stdout", "-s", ESPEAK_RATE, "-v", espeak_voice, "--stdin"]);
        let (status, bytes) = super::writer::run(command, text, generation)?;
        if !status.success() {
            tracing::warn!(?status, "espeak-ng could not read a sentence");
            return None;
        }
        return super::wav::samples(&bytes);
    }
    if let Some(spd_say) = &installed.spd_say {
        SPEECH_DISPATCHER.store(true, Ordering::Release);
        let mut command = crate::desktop::background_process::command(spd_say);
        // `-w` waits until it's read, so sentences follow each other.
        command.args(["-w", "-l", &language, "--", text]);
        let _ = super::writer::run(command, "", generation);
        return None;
    }
    tracing::warn!("no voice is installed (Piper, espeak-ng, or speech-dispatcher)");
    None
}

fn piper_write(piper: &Path, model: &Path, text: &str, generation: u64) -> Option<Vec<f32>> {
    let file = super::writer::temp_file("wav");
    let mut command = crate::desktop::background_process::command(piper);
    command.arg("-m").arg(model).arg("-f").arg(&file);
    let status = super::writer::run(command, text, generation).map(|(status, _)| status);
    super::writer::take_file(&file, status)
}

/// Makes Piper's voice folder and opens the page its voices download from:
/// each is a `.onnx` model with its `.onnx.json`, put in that folder.
pub fn open_voice_settings() {
    let folder = dirs::data_dir().map(|data| data.join("piper/voices"));
    if let Some(folder) = &folder
        && let Err(error) = std::fs::create_dir_all(folder)
    {
        tracing::warn!(%error, "could not make Piper's voice folder");
    }
    let opened = crate::desktop::background_process::command("xdg-open")
        .arg("https://huggingface.co/rhasspy/piper-voices/tree/main")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        });
    if let Err(error) = opened {
        tracing::warn!(%error, "could not open Piper's voices");
    }
}
