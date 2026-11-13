//! "Stopp", said instead of a question: Agent-Desk goes quiet instead of asking.
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
    "bitte", "danke", "ok", "okay", "hey", "agent", "desk", "jetzt", "mal", "sofort", "please",
    "now", "thanks", "ja", "yes", "nein", "no", "einfach",
];

/// Phrases that stop every task.
const ALL: &[&str] = &[
    "stopp alle aufgaben",
    "stopp alles",
    "stop alles",
    "alles stoppen",
    "alle aufgaben stoppen",
    "alle aufgaben abbrechen",
    // The glossary's word for a job, as the window says it.
    "stopp alle aufträge",
    "alle aufträge stoppen",
    "alle aufträge abbrechen",
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

/// Verbs that open a stop of one task: "Brich die Shop-Aufgabe ab",
/// "Stopp den Einstein-Auftrag", "Cancel the supplier job".
const STOP_ONE: &[&str] = &[
    "stopp",
    "stoppe",
    "stop",
    "brich",
    "beende",
    "cancel",
    "abbrechen",
];
/// Words naming what's stopped, never part of its name.
const NOT_A_NAME: &[&str] = &[
    "die", "den", "das", "der", "dem", "the", "a", "an", "ab", "aufgabe", "auftrag", "job", "task",
    "bitte", "please", "mal", "jetzt", "sofort", "now",
];
/// Words that make a stop one of a job, not of something else the person
/// names ("Stopp das Video" pauses a video; "Stopp die Video-Aufgabe"
/// stops the job).
const TASK_WORDS: &[&str] = &["aufgabe", "auftrag", "job", "task"];

/// The words naming one task when `text` asks to stop it ("Brich die
/// Shop-Aufgabe ab" → ["shop"]); `None` when it's no such stop. A plain stop
/// ([`spoken_stop`]) and stopping everything are not.
pub(crate) fn named_stop(text: &str) -> Option<Vec<String>> {
    if spoken_stop(text).is_some() {
        return None;
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.len() > 8 || !STOP_ONE.contains(&words.first()?.as_str()) {
        return None;
    }
    if !words.iter().any(|word| TASK_WORDS.contains(&word.as_str())) {
        return None;
    }
    let names: Vec<String> = words[1..]
        .iter()
        .filter(|word| !NOT_A_NAME.contains(&word.as_str()) && !STOP_ONE.contains(&word.as_str()))
        .cloned()
        .collect();
    (!names.is_empty()).then_some(names)
}

/// The one running task whose title has a word (four letters or more)
/// that `names` holds; `None` when none or several do.
pub(crate) fn task_named<'a>(
    names: &[String],
    tasks: &'a [crate::assistant::TaskStatus],
) -> Option<&'a crate::assistant::TaskStatus> {
    let matches = |task: &&crate::assistant::TaskStatus| {
        task.title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .any(|word| {
                names
                    .iter()
                    .any(|name| name.chars().count() >= 4 && word == name)
            })
    };
    let mut found = tasks.iter().filter(matches);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(id: &str, title: &str) -> crate::assistant::TaskStatus {
        crate::assistant::TaskStatus {
            conversation: id.into(),
            origin: crate::assistant::Origin::Background,
            title: title.into(),
            state: crate::assistant::TaskState::Running,
            step: None,
            windows: Vec::new(),
            colour: 0,
        }
    }

    #[test]
    fn one_task_is_stopped_by_its_name() {
        let tasks = [
            status("shop", "Online-Shop wie Nike bauen"),
            status("einstein", "Einstein recherchieren"),
        ];
        for (said, id) in [
            ("Brich die Shop-Aufgabe ab", "shop"),
            ("Stopp den Einstein Auftrag", "einstein"),
            ("Cancel the shop task", "shop"),
            ("Beende bitte den Einstein-Job", "einstein"),
        ] {
            let names = named_stop(said).unwrap_or_else(|| panic!("{said}"));
            assert_eq!(
                task_named(&names, &tasks).map(|t| t.conversation.as_str()),
                Some(id),
                "{said}"
            );
        }
        // A plain stop, everything, or a request that isn't a stop.
        assert_eq!(named_stop("Stopp"), None);
        assert_eq!(named_stop("Stopp alle Aufgaben"), None);
        assert_eq!(named_stop("Stopp alle Aufträge"), None);
        assert_eq!(named_stop("Stoppuhr auf fünf Minuten"), None);
        assert_eq!(named_stop("Halte mir den Termin frei"), None);
        // A stop of something else, even with a job's word in it, is no
        // job's stop: „Stopp das Video“ pauses a video.
        assert_eq!(named_stop("Stopp das Video"), None);
        assert_eq!(named_stop("Cancel my meeting with Anna"), None);
        // A stop that names no running task stays a request.
        let names = named_stop("Beende die Präsentations-Aufgabe").unwrap();
        assert!(task_named(&names, &tasks).is_none());
        // Two tasks named alike: not guessed.
        let twins = [status("a", "Shop bauen"), status("b", "Shop testen")];
        assert!(task_named(&["shop".into()], &twins).is_none());
    }

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
            "Hey Agent-Desk, Ruhe!",
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
            "Stopp alle Aufträge.",
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
