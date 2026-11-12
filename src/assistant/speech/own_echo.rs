//! What Hex read aloud lately, so a microphone that heard it back through
//! the speakers isn't taken for the person. Conversation mode listens right
//! after an answer; if the reading ran longer than it looked (a late first
//! sentence, a gap between sentences), the capture holds Hex's own words.
//! Such a transcript is dropped instead of asked.

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

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
