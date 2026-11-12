//! Supertonic's text front end: the text as the model reads it (normalized,
//! tagged with its language) and the character ids it takes.
//!
//! Ported from Supertone's MIT-licensed Rust example
//! (`rust/src/helper.rs` in github.com/supertone-inc/supertonic):
//! Copyright (c) 2025 Supertone Inc. Permission is hereby granted, free of
//! charge, to any person obtaining a copy of this software and associated
//! documentation files, to deal in the Software without restriction,
//! subject to including this notice; the software is provided "as is",
//! without warranty of any kind (MIT License, full text in
//! THIRD_PARTY_NOTICES.md).

use unicode_normalization::UnicodeNormalization;

/// The languages Supertonic 3 reads (`AVAILABLE_LANGS` upstream, without
/// its `na` placeholder).
pub const LANGUAGES: &[&str] = &[
    "en", "ko", "ja", "ar", "bg", "cs", "da", "de", "el", "es", "et", "fi", "fr", "hi", "hr", "hu",
    "id", "it", "lt", "lv", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "tr", "uk", "vi",
];

/// What the voice reads, unheard, to warm up after loading.
pub const WARM_UP: &str = "Hallo, ich bin bereit.";

/// Supertonic's code for `language` (`de`, `de-DE`, `pt_BR`), if it reads it.
pub fn language_code(language: &str) -> Option<&'static str> {
    let base = language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    LANGUAGES.iter().copied().find(|code| *code == base)
}

/// `text` as the model reads it in `language` (one of [`LANGUAGES`]):
/// decomposed, without emoji and symbols it can't say, with spacing fixed,
/// ending in punctuation, and wrapped in language tags (`<de>…</de>`).
pub fn prepare(text: &str, language: &str) -> String {
    let mut text: String = text.nfkd().filter(|c| !is_emoji(*c)).collect();
    for (from, to) in [
        ("–", "-"),
        ("‑", "-"),
        ("—", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("´", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("→", " "),
        ("←", " "),
    ] {
        text = text.replace(from, to);
    }
    for symbol in ["♥", "☆", "♡", "©", "\\"] {
        text = text.replace(symbol, "");
    }
    for (from, to) in [
        ("@", " at "),
        ("e.g.,", "for example, "),
        ("i.e.,", "that is, "),
    ] {
        text = text.replace(from, to);
    }
    for mark in [",", ".", "!", "?", ";", ":", "'"] {
        text = text.replace(&format!(" {mark}"), mark);
    }
    for doubled in ["\"\"", "''", "``"] {
        let single = &doubled[..1];
        while text.contains(doubled) {
            text = text.replace(doubled, single);
        }
    }
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text
        .chars()
        .last()
        .is_some_and(|last| !ENDINGS.contains(last))
    {
        text.push('.');
    }
    format!("<{language}>{text}</{language}>")
}

/// What may end a sentence without a period being added.
const ENDINGS: &str = ".!?;:,'\"\u{201C}\u{201D}\u{2018}\u{2019})]}…。」』】〉》›»";

/// The emoji ranges upstream removes.
fn is_emoji(c: char) -> bool {
    matches!(
        c,
        '\u{1F600}'..='\u{1F64F}'
            | '\u{1F300}'..='\u{1F5FF}'
            | '\u{1F680}'..='\u{1F6FF}'
            | '\u{1F700}'..='\u{1F77F}'
            | '\u{1F780}'..='\u{1F7FF}'
            | '\u{1F800}'..='\u{1F8FF}'
            | '\u{1F900}'..='\u{1F9FF}'
            | '\u{1FA00}'..='\u{1FA6F}'
            | '\u{1FA70}'..='\u{1FAFF}'
            | '\u{2600}'..='\u{26FF}'
            | '\u{2700}'..='\u{27BF}'
            | '\u{1F1E6}'..='\u{1F1FF}'
    )
}

/// The model's id for each character of `prepared`, by `unicode_indexer`
/// (indexed by code point); -1 for a character past its end.
pub fn ids(prepared: &str, indexer: &[i64]) -> Vec<i64> {
    prepared
        .chars()
        .map(|c| indexer.get(c as usize).copied().unwrap_or(-1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_tagged_spaced_and_ends_in_punctuation() {
        assert_eq!(
            prepare("Es ist  18 Uhr  in Tokio", "de"),
            "<de>Es ist 18 Uhr in Tokio.</de>"
        );
        assert_eq!(prepare("Fertig !", "de"), "<de>Fertig!</de>");
        assert_eq!(
            prepare("„Hallo“ – sagte sie 😀", "de"),
            "<de>„Hallo\" - sagte sie.</de>"
        );
        assert_eq!(prepare("a/b [c]", "en"), "<en>a b c.</en>");
    }

    #[test]
    fn umlauts_are_decomposed_as_the_model_was_trained() {
        // NFKD: ü is u and a combining diaeresis.
        assert_eq!(prepare("Grüße.", "de"), "<de>Gru\u{308}ße.</de>");
    }

    #[test]
    fn characters_map_through_the_indexer() {
        let mut indexer = vec![0_i64; 128];
        indexer['a' as usize] = 5;
        indexer['.' as usize] = 9;
        assert_eq!(ids("a.", &indexer), [5, 9]);
        assert_eq!(ids("aé", &indexer), [5, -1]);
    }

    #[test]
    fn languages_map_to_supertonics_codes() {
        assert_eq!(language_code("de"), Some("de"));
        assert_eq!(language_code("de-DE"), Some("de"));
        assert_eq!(language_code("pt_BR"), Some("pt"));
        assert_eq!(language_code("EN"), Some("en"));
        assert_eq!(language_code("zh"), None);
        assert_eq!(language_code("na"), None);
    }
}
