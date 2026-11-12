//! How Windows' and Linux's engines list their voices, read into
//! [`Listed`] voices. Kept apart from the engines so it is tested on every
//! OS.

#[cfg(any(test, target_os = "linux"))]
use std::path::Path;

use super::voices::{Listed, Quality};

/// What a Linux voice id starts with: the engine that reads it.
#[cfg(any(test, target_os = "linux"))]
pub const PIPER: &str = "piper:";
#[cfg(any(test, target_os = "linux"))]
pub const ESPEAK: &str = "espeak:";

/// A voice from `Windows.Media.SpeechSynthesis` ("Microsoft Katja",
/// `de-DE`). Its natural voices ("… Online (Natural)") read best.
#[cfg(any(test, target_os = "windows"))]
pub fn windows_listed(id: &str, display_name: &str, language: &str) -> Listed {
    let lower = display_name.to_lowercase();
    let quality = if lower.contains("natural") {
        Quality::Premium
    } else if lower.contains("online") {
        Quality::Enhanced
    } else {
        Quality::Default
    };
    let name = display_name
        .trim()
        .strip_prefix("Microsoft ")
        .unwrap_or(display_name)
        .trim_end_matches(" Desktop");
    Listed {
        id: id.to_owned(),
        name: name.to_owned(),
        locale: language.to_owned(),
        quality,
    }
}

/// `espeak-ng --voices`' table: one language a line after the header,
/// `Pty Language Age/Gender VoiceName File Other Languages`
/// (` 5  de  --/M  German  gmw/de`). A voice is read by its language
/// (`-v de`).
#[cfg(any(test, target_os = "linux"))]
pub fn espeak_voices(output: &str) -> Vec<Listed> {
    output
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let priority = columns.next()?;
            priority.parse::<u32>().ok()?;
            let language = columns.next()?;
            let _gender = columns.next()?;
            let name = columns.next()?.replace('_', " ");
            Some(Listed {
                id: format!("{ESPEAK}{language}"),
                name,
                locale: language.to_owned(),
                quality: Quality::Default,
            })
        })
        .collect()
}

/// A Piper voice model (`de_DE-thorsten-high.onnx`, with its
/// `….onnx.json` beside it): named after its speaker, `high` models read
/// best. `None` for a file that isn't named like one.
#[cfg(any(test, target_os = "linux"))]
pub fn piper_voice(model: &Path) -> Option<Listed> {
    let stem = model.file_name()?.to_str()?.strip_suffix(".onnx")?;
    let mut parts = stem.splitn(3, '-');
    let locale = parts.next()?;
    let speaker = parts.next()?;
    let size = parts.next().unwrap_or("medium");
    let looks_like_locale = locale.len() >= 2
        && locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !looks_like_locale || speaker.is_empty() {
        return None;
    }
    let quality = match size {
        "high" => Quality::Premium,
        _ => Quality::Enhanced,
    };
    let mut name = String::new();
    for (index, word) in speaker
        .split('_')
        .filter(|word| !word.is_empty())
        .enumerate()
    {
        if index > 0 {
            name.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.extend(first.to_uppercase());
            name.push_str(chars.as_str());
        }
    }
    Some(Listed {
        id: format!("{PIPER}{}", model.display()),
        name,
        locale: locale.to_owned(),
        quality,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::speech::voices::{self, Voice};

    const ESPEAK_LIST: &str =
        "Pty Language       Age/Gender VoiceName          File                 Other Languages
 5  af              --/M      Afrikaans          gmw/af
 5  de              --/M      German             gmw/de
 2  en-gb           --/M      English_(Great_Britain) gmw/en          (en 2)
 5  en-us           --/M      English_(America)  gmw/en-US            (en 3)
";

    #[test]
    fn windows_voices_are_named_without_the_vendor() {
        let katja = windows_listed("k", "Microsoft Katja", "de-DE");
        assert_eq!(
            (katja.name.as_str(), katja.quality),
            ("Katja", Quality::Default)
        );
        let zira = windows_listed("z", "Microsoft Zira Desktop", "en-US");
        assert_eq!(zira.name, "Zira");
        assert_eq!(
            windows_listed("n", "Microsoft Katja Online (Natural)", "de-DE").quality,
            Quality::Premium
        );
    }

    #[test]
    fn espeak_lists_one_voice_a_language() {
        let listed = espeak_voices(ESPEAK_LIST);
        assert_eq!(listed.len(), 4);
        assert_eq!(listed[1].id, "espeak:de");
        assert_eq!(listed[1].name, "German");
        assert_eq!(listed[2].name, "English (Great Britain)");
        assert_eq!(listed[2].locale, "en-gb");
        let voices: Vec<Voice> = voices::named(listed);
        assert_eq!(voices[2].name, "English (en-gb)");
        assert_eq!(voices[3].name, "English (en-us)");
        assert_eq!(voices[3].language(), "en");
        assert!(espeak_voices("nonsense\n\n").is_empty());
    }

    #[test]
    fn piper_models_are_named_after_their_speaker() {
        let thorsten = piper_voice(Path::new("/v/de_DE-thorsten-high.onnx")).unwrap();
        assert_eq!(thorsten.id, "piper:/v/de_DE-thorsten-high.onnx");
        assert_eq!(thorsten.name, "Thorsten");
        assert_eq!(thorsten.locale, "de_DE");
        assert_eq!(thorsten.quality, Quality::Premium);
        let lessac = piper_voice(Path::new("en_US-libritts_r-medium.onnx")).unwrap();
        assert_eq!(
            (lessac.name.as_str(), lessac.quality),
            ("Libritts R", Quality::Enhanced)
        );
        assert_eq!(piper_voice(Path::new("model.onnx")), None);
        assert_eq!(
            piper_voice(Path::new("de_DE-thorsten-high.onnx.json")),
            None
        );
    }

    /// Piper's voices beat espeak-ng's, and among them `high` ones.
    #[test]
    fn piper_reads_before_espeak() {
        let mut listed = espeak_voices(ESPEAK_LIST);
        listed.push(piper_voice(Path::new("de_DE-thorsten-medium.onnx")).unwrap());
        listed.push(piper_voice(Path::new("de_DE-kerstin-low.onnx")).unwrap());
        let installed = voices::named(listed);
        let best = voices::best_voice("de", "de_DE", &installed).map(|voice| &*voice.name);
        assert_eq!(best, Some("Kerstin (Enhanced)"));
        let english = voices::best_voice("en", "en_US", &installed).map(|voice| &*voice.id);
        assert_eq!(english, Some("espeak:en-us"));
    }
}
