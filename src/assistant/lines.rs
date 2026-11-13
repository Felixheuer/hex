//! What the assistant says or shows back to a request (a steer note, "On
//! it", a status, a refusal) in the language of what was said, through the
//! interface's translations (`desktop::i18n`), never a word list of its own.
//! The German glossary (`docs/features/assistant.md`): a job is an
//! „Auftrag“, a quick turn a „Frage“; never „Aufgabe“ for a job.
//!
//! The names of the places it sends people to are here too, so every
//! message, menu, and button says the sidebar's names.

/// The assistant's pane in Hex's window: the stream of every quick turn
/// and job (`rune_ui::sidebar`).
pub const ASSISTANT: &str = "Assistant";
/// Its settings pane.
pub const ASSISTANT_SETTINGS: &str = "Assistant settings";

/// The language `said` is in, when its words tell (`speech::language_of`).
/// `None` keeps the interface's.
pub fn language(said: &str) -> Option<&'static str> {
    super::speech::language_of(said)
}

/// `english` in the language of `said`.
pub fn tr(english: &'static str, said: &str) -> &'static str {
    crate::desktop::i18n::tr_in(language(said), english)
}

/// A `{name}` template in the language of `said`, filled.
pub fn tr_named(template: &'static str, said: &str, values: &[(&str, &str)]) -> String {
    tr_named_in(language(said), template, values)
}

/// A `{name}` template in `language` (`desktop::i18n::tr_in`), filled.
pub fn tr_named_in(language: Option<&str>, template: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(
        crate::desktop::i18n::tr_in(language, template).to_owned(),
        |text, (name, value)| text.replace(&format!("{{{name}}}"), value),
    )
}

/// `place` (one of the constants above) as the interface names it, for a
/// message such as "Install it in Assistant settings".
pub fn place(place: &'static str) -> &'static str {
    crate::desktop::i18n::tr(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_follows_the_language_it_answers() {
        assert_eq!(
            tr(
                "Okay, I'll take that into account at the next step.",
                "Bitte nimm die blaue Farbe und nicht die rote"
            ),
            "Okay, ich berücksichtige das beim nächsten Schritt."
        );
        assert_eq!(
            tr(
                "Okay, I'll take that into account at the next step.",
                "Please use the blue one and not the red one"
            ),
            "Okay, I'll take that into account at the next step."
        );
    }

    #[test]
    fn the_places_have_every_language() {
        for place in [ASSISTANT, ASSISTANT_SETTINGS] {
            for language in ["de", "pl", "ja", "ru"] {
                assert_ne!(
                    crate::desktop::i18n::tr_in(Some(language), place),
                    place,
                    "{place} in {language}"
                );
            }
        }
    }
}
