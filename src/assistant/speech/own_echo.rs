//! What Hex read aloud lately, so a microphone that heard it back through
//! the speakers isn't taken for the person. Conversation mode listens right
//! after an answer; if the reading ran longer than it looked (a late first
//! sentence, a gap between sentences), the capture holds Hex's own words.
//! Such a transcript is dropped instead of asked.
//!
//! A capture that listened under the reading (barge-in, or conversation
//! mode's window) holds Hex's voice first and the person's words after it,
//! so [`without_own_words`] cuts the reading off the front: the transcript's
//! opening words are lined up, in order, with what Hex read, and what follows
//! the last of them is the person's.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a spoken text counts as recent.
const RECENT: Duration = Duration::from_secs(90);
/// How many recent texts are kept.
const KEPT: usize = 16;
/// The share of a transcript's words that, found in what Hex just said,
/// make it Hex's own voice.
const OWN_SHARE: f32 = 0.7;
/// Transcripts shorter than this many words are never taken for an echo
/// ("ja", "stopp", "nochmal").
const MIN_WORDS: usize = 4;

static SPOKEN: Mutex<VecDeque<(Instant, String)>> = Mutex::new(VecDeque::new());

/// Notes `text` as read aloud now.
pub fn note(text: &str) {
    note_at(&SPOKEN, Instant::now(), text);
}

/// Whether `transcript` is mostly what Hex read aloud lately.
pub fn heard_back(transcript: &str) -> bool {
    heard_back_at(&SPOKEN, Instant::now(), transcript)
}

fn note_at(spoken: &Mutex<VecDeque<(Instant, String)>>, now: Instant, text: &str) {
    let mut spoken = spoken.lock().unwrap_or_else(|error| error.into_inner());
    spoken.retain(|(at, _)| now.duration_since(*at) < RECENT);
    spoken.push_back((now, text.to_owned()));
    while spoken.len() > KEPT {
        spoken.pop_front();
    }
}

fn heard_back_at(
    spoken: &Mutex<VecDeque<(Instant, String)>>,
    now: Instant,
    transcript: &str,
) -> bool {
    let heard = words(transcript);
    if heard.len() < MIN_WORDS {
        return false;
    }
    let spoken = spoken.lock().unwrap_or_else(|error| error.into_inner());
    let said: HashSet<String> = spoken
        .iter()
        .filter(|(at, _)| now.duration_since(*at) < RECENT)
        .flat_map(|(_, text)| words(text))
        .collect();
    if said.is_empty() {
        return false;
    }
    let found = heard.iter().filter(|word| said.contains(*word)).count();
    found as f32 / heard.len() as f32 >= OWN_SHARE
}

/// Words a transcript's opening may skip before Hex's reading is found in
/// it, and transcription misses forgiven in a row inside it.
const START_SKIP: usize = 2;
const MISSES: usize = 2;
/// How far ahead in the reading the next heard word may be (words the
/// microphone lost).
const LOOKAHEAD: usize = 4;
/// Words of the reading that must be found before any is cut.
const MIN_RUN: usize = 3;

/// `transcript` without Hex's own reading at its front (heard back through
/// the speakers before the person spoke): the person's words, or "" when
/// it held nothing else. Unchanged when it doesn't open with the reading.
pub fn without_own_words(transcript: &str) -> String {
    let spoken = SPOKEN.lock().unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    let said: Vec<String> = spoken
        .iter()
        .filter(|(at, _)| now.duration_since(*at) < RECENT)
        .flat_map(|(_, text)| words(text))
        .collect();
    drop(spoken);
    cut_reading(&said, transcript)
}

fn cut_reading(said: &[String], transcript: &str) -> String {
    let heard = spans(transcript);
    let mut at: Option<usize> = None;
    let (mut found, mut misses, mut cut) = (0usize, 0usize, None);
    for (index, (word, _, end)) in heard.iter().enumerate() {
        if neutral(word) {
            continue;
        }
        let next = match at {
            None => said.iter().position(|s| s == word),
            Some(from) => said[from.min(said.len())..]
                .iter()
                .take(LOOKAHEAD)
                .position(|s| s == word)
                .map(|offset| from + offset),
        };
        match next {
            Some(position) => {
                at = Some(position + 1);
                found += 1;
                misses = 0;
                cut = Some(*end);
            }
            None if at.is_none() => {
                if index >= START_SKIP {
                    break;
                }
            }
            None => {
                misses += 1;
                if misses > MISSES {
                    break;
                }
            }
        }
    }
    match cut {
        Some(end) if found >= MIN_RUN => transcript[end..]
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim_end()
            .to_owned(),
        _ => transcript.trim().to_owned(),
    }
}

/// Each word of `text` (as [`words`] keeps them) with its byte span.
fn spans(text: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                out.push((text[from..index].to_lowercase(), from, index));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// A number or a number's word: it comes back spelled out or not, so it
/// neither matches nor misses.
fn neutral(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&word)
}

/// Numbers come back spelled out ("1,9" heard as "eins komma neun"), so
/// they and their words never count either way.
const NUMBER_WORDS: &[&str] = &[
    "null", "eins", "ein", "eine", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht",
    "neun", "zehn", "elf", "zwölf", "zwanzig", "dreißig", "hundert", "tausend", "komma", "punkt",
    "prozent", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    "ten", "hundred", "thousand", "point", "percent",
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| {
            !word.chars().all(|c| c.is_ascii_digit()) && !NUMBER_WORDS.contains(&word.as_str())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str) -> Vec<String> {
        words(text)
    }

    #[test]
    fn the_reading_is_cut_off_the_front_and_the_person_s_words_stay() {
        let reading = said(
            "Hamburg ist mit rund 1,9 Millionen Einwohnern die zweitgrößte Stadt Deutschlands. Es liegt an der Elbe.",
        );
        // Barge-in: the reading so far, then the person.
        assert_eq!(
            cut_reading(
                &reading,
                "Hamburg ist mit rund eins komma neun Millionen Einwohnern die zweit größte Stadt. Und wie ist das Wetter dort?"
            ),
            "Und wie ist das Wetter dort?"
        );
        // Their question reuses the answer's words: still theirs.
        assert_eq!(
            cut_reading(
                &reading,
                "Hamburg ist mit rund 1,9 Millionen. Und in Berlin?"
            ),
            "Und in Berlin?"
        );
        // Only Hex's voice: nothing left, a false alarm.
        assert_eq!(
            cut_reading(&reading, "Hamburg ist mit rund 1,9 Millionen Einwohnern"),
            ""
        );
    }

    #[test]
    fn a_transcript_that_doesn_t_open_with_the_reading_stays_whole() {
        let reading = said("Die Hauptstadt von Frankreich ist Paris.");
        assert_eq!(
            cut_reading(&reading, "Was ist die Hauptstadt von Spanien?"),
            "Was ist die Hauptstadt von Spanien?"
        );
        assert_eq!(cut_reading(&reading, "Stopp"), "Stopp");
        assert_eq!(cut_reading(&[], "Öffne Safari"), "Öffne Safari");
        // Two words in a row aren't enough to cut.
        assert_eq!(
            cut_reading(&reading, "Die Hauptstadt? Erzähl mehr"),
            "Die Hauptstadt? Erzähl mehr"
        );
    }

    const ANSWER: &str = "Ich kann hier keinen Browser öffnen, weil ich in diesem Fenster nur antworten und Hex-Aufgaben ausführen darf. Starte den Browser bitte selbst.";

    #[test]
    fn hex_s_own_answer_heard_back_is_recognised() {
        let spoken = Mutex::new(VecDeque::new());
        let now = Instant::now();
        note_at(&spoken, now, ANSWER);
        // As the person's transcript came back: small transcription changes.
        assert!(heard_back_at(
            &spoken,
            now + Duration::from_secs(3),
            "Ich kann hier keinen Browser öffnen, weil ich in diesem Fenster nur Antworten und Hexaufgaben ausführen darf. Starte den Browser bitte selbst."
        ));
        // Only its tail, as a late capture would hold.
        assert!(heard_back_at(
            &spoken,
            now + Duration::from_secs(3),
            "Starte den Browser bitte selbst."
        ));
    }

    #[test]
    fn the_person_s_own_follow_ups_still_ask() {
        let spoken = Mutex::new(VecDeque::new());
        let now = Instant::now();
        note_at(&spoken, now, ANSWER);
        let later = now + Duration::from_secs(3);
        assert!(!heard_back_at(
            &spoken,
            later,
            "Doch, öffne bitte Safari mit nike.com"
        ));
        assert!(!heard_back_at(&spoken, later, "Stopp"));
        assert!(!heard_back_at(
            &spoken,
            later,
            "Warum kannst du keinen Browser öffnen?"
        ));
    }

    #[test]
    fn old_readings_are_forgotten() {
        let spoken = Mutex::new(VecDeque::new());
        let now = Instant::now();
        note_at(&spoken, now, ANSWER);
        assert!(!heard_back_at(
            &spoken,
            now + RECENT + Duration::from_secs(1),
            "Starte den Browser bitte selbst."
        ));
    }

    #[test]
    fn numbers_read_aloud_and_heard_spelled_out_still_match() {
        let spoken = Mutex::new(VecDeque::new());
        let now = Instant::now();
        note_at(
            &spoken,
            now,
            "Hamburg ist mit rund 1,9 Millionen Einwohnern die zweitgrößte Stadt Deutschlands.",
        );
        assert!(heard_back_at(
            &spoken,
            now + Duration::from_secs(2),
            "Hamburg ist mit rund eins komma neun Millionen."
        ));
    }
}
