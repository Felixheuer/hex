//! Stray Cyrillic words in a question spoken in a Latin-script language.
//!
//! Parakeet v3 hears the language itself and takes no hint, so a product
//! name in a German sentence can come back in Cyrillic ("Konfiguriere
//! Макмини мф про" for "Mac mini M5 Pro"). When most of the letters are
//! Latin, such words are spelled in Latin letters ("Makmini mf pro"), which
//! the assistant reads as meant. A question that is mostly Cyrillic is
//! Russian (or Ukrainian, …) and stays as it is.

use std::borrow::Cow;

/// Languages written in Cyrillic, whose questions keep their letters.
const CYRILLIC_LANGUAGES: &[&str] = &["ru", "uk", "be", "bg", "sr", "mk", "kk", "ky", "mn", "tg"];

/// `text` with its Cyrillic letters in Latin ones when it was spoken in a
/// Latin-script language: the one transcription is set to (`language`),
/// or, under Auto, when Latin letters are at least as many as Cyrillic ones.
/// Otherwise `text` unchanged.
pub(crate) fn latin_strays<'a>(text: &'a str, language: Option<&str>) -> Cow<'a, str> {
    let cyrillic = text.chars().filter(|c| is_cyrillic(*c)).count();
    if cyrillic == 0 {
        return Cow::Borrowed(text);
    }
    let language = language
        .map(|language| {
            language
                .split(['-', '_'])
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
        })
        .filter(|language| !language.is_empty() && language != "auto");
    let latin_language = match &language {
        Some(language) => !CYRILLIC_LANGUAGES.contains(&language.as_str()),
        None => {
            let latin = text
                .chars()
                .filter(|c| c.is_alphabetic() && !is_cyrillic(*c))
                .count();
            latin >= cyrillic
        }
    };
    if !latin_language {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match latin_for(c) {
            Some(latin) => out.push_str(latin),
            None => out.push(c),
        }
    }
    Cow::Owned(out)
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}')
}

/// The Latin spelling of a Cyrillic letter (Russian and Ukrainian ones);
/// `None` for any other character.
fn latin_for(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'ґ' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' | 'є' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' | 'й' | 'і' | 'ї' => "i",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'ю' => "yu",
        'я' => "ya",
        'А' => "A",
        'Б' => "B",
        'В' => "V",
        'Г' | 'Ґ' => "G",
        'Д' => "D",
        'Е' | 'Ё' | 'Э' | 'Є' => "E",
        'Ж' => "Zh",
        'З' => "Z",
        'И' | 'Й' | 'І' | 'Ї' => "I",
        'К' => "K",
        'Л' => "L",
        'М' => "M",
        'Н' => "N",
        'О' => "O",
        'П' => "P",
        'Р' => "R",
        'С' => "S",
        'Т' => "T",
        'У' => "U",
        'Ф' => "F",
        'Х' => "Kh",
        'Ц' => "Ts",
        'Ч' => "Ch",
        'Ш' => "Sh",
        'Щ' => "Shch",
        'Ъ' | 'Ь' => "",
        'Ы' => "Y",
        'Ю' => "Yu",
        'Я' => "Ya",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stray_cyrillic_name_in_a_german_question_is_spelled_in_latin() {
        for language in [Some("de"), None, Some("auto")] {
            assert_eq!(
                latin_strays("Konfiguriere Макмини мф про.", language),
                "Konfiguriere Makmini mf pro."
            );
        }
    }

    #[test]
    fn a_russian_question_stays_russian() {
        let russian = "Сколько сейчас времени в Токио?";
        assert_eq!(latin_strays(russian, Some("ru")), russian);
        assert_eq!(latin_strays(russian, None), russian);
        // A Latin brand name in a Russian sentence doesn't turn it around.
        let mixed = "Открой Safari и найди погоду в Москве";
        assert_eq!(latin_strays(mixed, Some("ru")), mixed);
        assert_eq!(latin_strays(mixed, None), mixed);
    }

    #[test]
    fn latin_only_text_is_untouched() {
        assert!(matches!(
            latin_strays("Wie spät ist es in Tokio?", Some("de")),
            Cow::Borrowed(_)
        ));
    }
}
