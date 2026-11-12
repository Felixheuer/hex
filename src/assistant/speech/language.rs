//! The language an answer is written in, and the system's own.

use std::sync::OnceLock;

/// Common words that tell these languages apart.
const MARKERS: [(&str, &[&str]); 6] = [
    (
        "de",
        &[
            "der", "die", "das", "und", "ist", "nicht", "ich", "du", "sie", "ein", "eine", "mit",
            "auf", "für", "es", "zu", "den", "dem", "wir", "bereit", "auch", "noch",
        ],
    ),
    (
        "en",
        &[
            "the", "and", "is", "are", "you", "it", "of", "to", "in", "that", "this", "with",
            "for", "not", "be", "your", "can",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "et", "est", "vous", "une", "des", "pas", "que", "pour", "avec",
        ],
    ),
    (
        "es",
        &[
            "el", "los", "las", "y", "es", "una", "que", "por", "para", "con", "está", "pero",
        ],
    ),
    (
        "it",
        &[
            "il", "lo", "gli", "e", "è", "una", "che", "per", "non", "sono", "con", "della",
        ],
    ),
    (
        "nl",
        &[
            "de", "het", "een", "en", "is", "niet", "ik", "je", "van", "dat", "met", "voor",
        ],
    ),
];

/// The language `text` is most likely in, by its common words (German also
/// by ä, ö, ü, ß); `None` when too little tells.
pub fn language_of(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut scores: Vec<(&'static str, usize)> = MARKERS
        .iter()
        .map(|(language, markers)| {
            let hits = words
                .iter()
                .filter(|word| markers.contains(&word.as_str()))
                .count();
            (*language, hits)
        })
        .collect();
    let umlauts = text
        .chars()
        .filter(|c| matches!(c, 'ä' | 'ö' | 'ü' | 'ß' | 'Ä' | 'Ö' | 'Ü'))
        .count();
    if let Some(german) = scores.iter_mut().find(|(language, _)| *language == "de") {
        german.1 += umlauts.min(3);
    }
    scores.sort_by_key(|(_, hits)| std::cmp::Reverse(*hits));
    match scores.as_slice() {
        [(language, best), (_, next), ..] if *best >= 1 && best > next => Some(language),
        _ => None,
    }
}

/// The system's first preferred language as a locale (`de-DE`), from
/// AppleLanguages.
pub fn system_locale() -> String {
    static LOCALE: OnceLock<String> = OnceLock::new();
    LOCALE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            let preferred = objc2::rc::autoreleasepool(|_| {
                objc2_foundation::NSLocale::preferredLanguages()
                    .firstObject()
                    .map(|language| language.to_string())
            });
            #[cfg(target_os = "windows")]
            let preferred = windows_locale();
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let preferred = ["LC_ALL", "LC_MESSAGES", "LANG"]
                .into_iter()
                .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()));
            preferred
                .map(|locale| {
                    locale
                        .split(['.', '@'])
                        .next()
                        .unwrap_or_default()
                        .replace('_', "-")
                })
                .filter(|locale| !locale.is_empty() && locale != "C" && locale != "POSIX")
                .unwrap_or_else(|| "en-US".into())
        })
        .clone()
}

/// The person's locale on Windows (`de-DE`), `GetUserDefaultLocaleName`.
#[cfg(target_os = "windows")]
fn windows_locale() -> Option<String> {
    let mut buffer = [0_u16; 85];
    // SAFETY: `buffer` holds LOCALE_NAME_MAX_LENGTH (85) characters.
    let length = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    let length = usize::try_from(length).ok().filter(|length| *length > 1)?;
    Some(String::from_utf16_lossy(&buffer[..length - 1]))
}

/// The system's language (`de` for a German Mac).
pub fn system_language() -> String {
    system_locale()
        .split(['-', '_'])
        .next()
        .unwrap_or("en")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_is_told_by_common_words() {
        assert_eq!(language_of("Hex Assistent ist bereit"), Some("de"));
        assert_eq!(
            language_of("Die Datei ist in deinem Downloads-Ordner."),
            Some("de")
        );
        assert_eq!(
            language_of("The file is in your Downloads folder."),
            Some("en")
        );
        assert_eq!(
            language_of("Le fichier est dans vos téléchargements et pas ici."),
            Some("fr")
        );
        assert_eq!(language_of("Größe: 12 MB"), Some("de"));
        assert_eq!(language_of("OK"), None);
        assert_eq!(language_of(""), None);
    }
}
