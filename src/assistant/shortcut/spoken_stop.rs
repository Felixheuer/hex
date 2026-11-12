//! "Stopp", said instead of a question: Hex goes quiet instead of asking.
//!
//! A hold or a conversation-mode follow-up whose words are only a stop
//! ("Stopp", "Halt", "Sei still", "Hör auf", "Stop", "Be quiet", …) ends the
//! answer being read aloud and the Here task working on it, and asks
//! nothing. "Stopp alle Aufgaben" / "Stop all tasks" stops every task, as
//! the Stop all shortcut does. Anything longer is a request as usual, so
//! "Stoppuhr auf fünf Minuten" or "Halte mir den Termin frei" still ask.

/// What a stop said aloud asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpokenStop {
    /// Quiet: the speech and the Here tasks.
    Quiet,
    /// Every task and the speech.
    All,
}

/// Words that alone (with politeness around them) mean "be quiet".
const QUIET: &[&str] = &[
    "stopp",
    "stop",
    "halt",
    "ruhe",
    "still",
    "sei still",
    "sei ruhig",
    "seid still",
    "hör auf",
    "hoer auf",
    "aufhören",
    "aufhoeren",
    "schluss",
    "abbrechen",
    "abbruch",
    "genug",
    "das reicht",
    "danke das reicht",
    "be quiet",
    "quiet",
    "shut up",
    "cancel",
    "enough",
    "that's enough",
    "thats enough",
];

/// Words around a stop that don't change it.
const FILLER: &[&str] = &[
    "bitte", "danke", "ok", "okay", "hey", "hex", "jetzt", "mal", "sofort", "please", "now",
    "thanks", "ja", "yes", "nein", "no", "einfach",
];

/// Phrases that stop every task.
const ALL: &[&str] = &[
    "stopp alle aufgaben",
    "stopp alles",
    "stop alles",
    "alles stoppen",
    "alle aufgaben stoppen",
    "alle aufgaben abbrechen",
    "brich alles ab",
    "stop all tasks",
    "stop all",
    "stop everything",
    "cancel everything",
];

/// The stop `text` says, if it says nothing but a stop.
pub(crate) fn spoken_stop(text: &str) -> Option<SpokenStop> {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() || words.len() > 6 {
        return None;
    }
    let kept: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|word| !FILLER.contains(word))
        .collect();
    let phrase = kept.join(" ");
    if ALL.contains(&phrase.as_str()) {
        return Some(SpokenStop::All);
    }
    if QUIET.contains(&phrase.as_str()) {
        return Some(SpokenStop::Quiet);
    }
    // "Stopp stopp", "Halt, stopp": only stop words.
    (!kept.is_empty() && kept.iter().all(|word| QUIET.contains(word))).then_some(SpokenStop::Quiet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_alone_means_quiet() {
        for said in [
            "Stopp.",
            "Stopp!",
            "stop",
            "Halt",
            "Sei still.",
            "Hör auf",
            "Okay, stopp bitte.",
            "Danke, das reicht.",
            "Stopp, stopp",
            "Hey Hex, Ruhe!",
            "Be quiet, please.",
            "That's enough.",
            "Abbrechen",
        ] {
            assert_eq!(spoken_stop(said), Some(SpokenStop::Quiet), "{said}");
        }
    }

    #[test]
    fn stopping_everything_is_its_own_phrase() {
        for said in [
            "Stopp alle Aufgaben.",
            "Bitte alles stoppen",
            "Brich alles ab!",
            "Stop all tasks",
            "Stop everything.",
        ] {
            assert_eq!(spoken_stop(said), Some(SpokenStop::All), "{said}");
        }
    }

    #[test]
    fn requests_that_merely_start_like_a_stop_still_ask() {
        for said in [
            "Stoppuhr auf fünf Minuten",
            "Halte mir Freitag frei",
            "Stopp die Musik in Spotify und öffne Mail",
            "Wie stoppe ich den Timer?",
            "Ruhezeiten in Berlin",
            "Schluss mit lustig, schreib Maya",
            "",
        ] {
            assert_eq!(spoken_stop(said), None, "{said}");
        }
    }
}
