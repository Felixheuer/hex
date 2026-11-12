//! Linux's speech programs, best first: Piper with a downloaded voice
//! (natural; `piper` on PATH and `<locale>-<name>-<quality>.onnx` voices in
//! `~/.local/share/piper-voices`, `~/.local/share/piper`,
//! `/usr/share/piper-voices`), Pico (`pico2wave`, six languages), and
//! eSpeak NG (`espeak-ng` or `espeak`, every language, robotic). A voice's
//! id says which program reads it: `piper:<model>`, `pico:<locale>`,
//! `espeak:<voice>`.

use std::path::{Path, PathBuf};

use super::super::voices::{self, Listed, Quality, Voice};
use super::{Job, Plan};

/// Pico's languages.
const PICO: [&str; 6] = ["de-DE", "en-US", "en-GB", "es-ES", "fr-FR", "it-IT"];
/// eSpeak's words a minute: a little brisker than its 175.
const ESPEAK_WPM: &str = "180";

/// The program for `job`'s voice writing it to `file`.
pub fn plan(job: &Job, file: &Path) -> Plan {
    let voice = job.voice.as_deref().unwrap_or_default();
    if let Some(model) = voice.strip_prefix("piper:")
        && let Some(piper) = program("piper")
    {
        let mut command = crate::desktop::background_process::command(piper);
        command
            .arg("--model")
            .arg(model)
            .arg("--output_file")
            .arg(file);
        return Plan::Command {
            command,
            stdin: Some(job.text.clone()),
        };
    }
    if let Some(locale) = voice.strip_prefix("pico:")
        && let Some(pico) = program("pico2wave")
    {
        let mut command = crate::desktop::background_process::command(pico);
        // pico2wave takes the text only as an argument; "--" keeps a text
        // that starts with "-" from being an option.
        command
            .args(["-l", locale, "-w"])
            .arg(file)
            .arg("--")
            .arg(&job.text);
        return Plan::Command {
            command,
            stdin: None,
        };
    }
    let Some(espeak) = espeak() else {
        return Plan::Audio(None);
    };
    let mut command = crate::desktop::background_process::command(espeak);
    if let Some(name) = voice.strip_prefix("espeak:") {
        command.args(["-v", name]);
    }
    command
        .args(["-s", ESPEAK_WPM, "-b", "1", "-w"])
        .arg(file)
        .arg("--stdin");
    Plan::Command {
        command,
        stdin: Some(job.text.clone()),
    }
}

/// Every voice the installed programs read: Piper's downloaded voices,
/// Pico's languages, and eSpeak's for the languages neither has.
pub fn voices() -> Vec<Voice> {
    let mut listed = Vec::new();
    if program("piper").is_some() {
        listed.extend(piper_voices(&piper_dirs()));
    }
    if program("pico2wave").is_some() {
        listed.extend(PICO.iter().map(|locale| Listed {
            id: format!("pico:{locale}"),
            name: "Pico".into(),
            locale: (*locale).into(),
            quality: Quality::Default,
        }));
    }
    if let Some(espeak) = espeak() {
        let output = crate::desktop::background_process::command(espeak)
            .arg("--voices")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(output) = output {
            let covered: Vec<String> = listed.iter().map(|voice| language(&voice.locale)).collect();
            listed.extend(
                espeak_voices(&String::from_utf8_lossy(&output.stdout))
                    .into_iter()
                    .filter(|voice| !covered.contains(&language(&voice.locale))),
            );
        }
    }
    voices::named(listed)
}

fn language(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn program(name: &str) -> Option<PathBuf> {
    crate::assistant::backends::search::find(name)
}

fn espeak() -> Option<PathBuf> {
    program("espeak-ng").or_else(|| program("espeak"))
}

/// Where Piper's voices are downloaded to.
fn piper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        let data = PathBuf::from(data);
        dirs.push(data.join("piper-voices"));
        dirs.push(data.join("piper"));
    }
    if let Some(home) = crate::app_paths::home_dir() {
        dirs.push(home.join(".local/share/piper-voices"));
        dirs.push(home.join(".local/share/piper"));
    }
    dirs.push("/usr/share/piper-voices".into());
    dirs.push("/usr/share/piper".into());
    dirs.dedup();
    dirs
}

/// The Piper voices in `dirs` (and one level of folders in them): each
/// `.onnx` model with its `.onnx.json` beside it, named
/// `<locale>-<name>-<quality>.onnx` as Piper publishes them.
fn piper_voices(dirs: &[PathBuf]) -> Vec<Listed> {
    let mut models = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(inner) = std::fs::read_dir(&path) {
                    models.extend(inner.flatten().map(|entry| entry.path()));
                }
            } else {
                models.push(path);
            }
        }
    }
    models.sort();
    models.dedup();
    models
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "onnx")
        })
        .filter(|path| path.with_extension("onnx.json").is_file())
        .filter_map(|path| piper_voice(&path))
        .collect()
}

/// `de_DE-thorsten-high.onnx` → Thorsten, de_DE, Premium.
fn piper_voice(model: &Path) -> Option<Listed> {
    let stem = model.file_stem()?.to_str()?;
    let mut parts = stem.splitn(3, '-');
    let locale = parts.next()?;
    let name = parts.next()?;
    let quality = parts.next().unwrap_or("medium");
    if locale.len() < 2 || name.is_empty() {
        return None;
    }
    let mut shown = name.replace('_', " ");
    if let Some(first) = shown.get(..1) {
        shown = first.to_uppercase() + &shown[1..];
    }
    Some(Listed {
        id: format!("piper:{}", model.display()),
        name: shown,
        locale: locale.to_owned(),
        quality: match quality {
            "high" | "medium" => Quality::Premium,
            _ => Quality::Enhanced,
        },
    })
}

/// `espeak-ng --voices`' table: `Pty Language Age/Gender VoiceName File
/// Other Languages`, one voice a line.
fn espeak_voices(output: &str) -> Vec<Listed> {
    let mut seen = Vec::new();
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let _priority = columns.next()?;
            let language = columns.next()?;
            let _gender = columns.next()?;
            let _name = columns.next()?;
            // Accents and dialects (`vi-vn-x-south`) only crowd the list.
            if language.contains("-x-") || seen.iter().any(|known: &String| known == language) {
                return None;
            }
            seen.push(language.to_owned());
            Some(Listed {
                id: format!("espeak:{language}"),
                name: "eSpeak".into(),
                locale: language.to_owned(),
                quality: Quality::Default,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piper_voices_are_named_from_their_files() {
        let voice = piper_voice(Path::new("/v/de_DE-thorsten-high.onnx")).expect("a voice");
        assert_eq!(voice.name, "Thorsten");
        assert_eq!(voice.locale, "de_DE");
        assert_eq!(voice.quality, Quality::Premium);
        assert_eq!(voice.id, "piper:/v/de_DE-thorsten-high.onnx");
        let low = piper_voice(Path::new("en_US-lessac-low.onnx")).expect("a voice");
        assert_eq!(low.quality, Quality::Enhanced);
        assert!(piper_voice(Path::new("model.onnx")).is_none());
    }

    #[test]
    fn espeak_lists_one_voice_a_language() {
        let output = "Pty Language       Age/Gender VoiceName          File                 Other Languages\n \
                      5  de              --/M      German             gmw/de\n \
                      5  en-us           --/M      American_English   gmw/en-US            (en 3)\n \
                      5  de              --/F      German_f           gmw/de-f\n";
        let voices = espeak_voices(output);
        let ids: Vec<&str> = voices.iter().map(|voice| voice.id.as_str()).collect();
        assert_eq!(ids, ["espeak:de", "espeak:en-us"]);
        assert_eq!(voices[1].locale, "en-us");
    }

    #[test]
    fn a_chosen_voice_picks_its_program() {
        let job = |voice: &str| Job {
            id: 1,
            text: "-hallo".into(),
            voice: Some(voice.into()),
        };
        // Without the programs installed nothing is asked to read; with
        // them, the text never becomes an option.
        if let Plan::Command { command, stdin } = plan(&job("espeak:de"), Path::new("/t/a.wav")) {
            let args: Vec<_> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            assert!(args.contains(&"--stdin".to_owned()), "{args:?}");
            assert_eq!(stdin.as_deref(), Some("-hallo"));
        }
    }
}

/// Live, read-only (no audio plays): this computer's voices, and a sentence
/// written by the best German one.
#[cfg(test)]
mod live {
    #[test]
    #[ignore = "reads this computer's speech programs"]
    fn this_computers_voices_write_a_sentence() {
        let voices = super::voices();
        eprintln!("{voices:#?}");
        let Some(voice) = super::super::super::voices::best_voice("de", "de-DE", &voices) else {
            return;
        };
        let job = super::Job {
            id: 7,
            text: "Hallo, ich bin dein Assistent.".into(),
            voice: Some(voice.id.clone()),
        };
        let samples = super::super::write(
            &job,
            super::super::GENERATION.load(std::sync::atomic::Ordering::Acquire),
        )
        .expect("audio");
        eprintln!("{} with {} samples", voice.name, samples.len());
        assert!(samples.len() > 48_000 / 2);
    }
}
