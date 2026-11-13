//! The person's preferences, learned from their corrections: "nicht so
//! förmlich", "schick das künftig immer an max@firma.de", "always save
//! invoices in the folder Belege". Each becomes one line of the memory
//! note (`memory.md`), which every later task reads:
//!
//! ```text
//! - Preference (tone): less formal — learned from “nicht so förmlich”
//! - Preference (recipient): max@firma.de — learned from “… immer an max@firma.de …”
//! ```
//!
//! No confirmation is asked (a correction is the person's own words, and
//! only their own requests are read, never what a task read or wrote), but
//! every entry is marked as learned and stays in the note, where the person
//! reviews, edits, or deletes it in Settings. A newer tone replaces the
//! one it contradicts ("more formal" after "less formal"). Nothing secret
//! is kept (`memory::looks_secret`), and only lasting wishes count: a
//! recipient or folder needs a word like "immer", "künftig", "always", or
//! "from now on"; a tone correction is about style and counts as said.
//!
//! OWNER: brain.

use std::sync::OnceLock;

use regex::Regex;

/// What a preference is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tone,
    Recipient,
    Folder,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Tone => "tone",
            Self::Recipient => "recipient",
            Self::Folder => "folder",
        }
    }
}

/// One preference found in the person's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preference {
    pub kind: Kind,
    pub value: String,
    /// The words it was learned from, shortened.
    pub said: String,
}

/// Tones, each with the words that ask for it and the tone it replaces.
const TONES: &[(&str, &[&str], &str)] = &[
    (
        "less formal",
        &[
            "nicht so förmlich",
            "weniger förmlich",
            "nicht so formell",
            "weniger formell",
            "nicht so steif",
            "lockerer",
            "zu förmlich",
            "zu formell",
            "less formal",
            "not so formal",
            "too formal",
            "more casual",
            "more relaxed",
            "less stiff",
        ],
        "more formal",
    ),
    (
        "more formal",
        &[
            "förmlicher",
            "formeller",
            "seriöser",
            "zu locker",
            "nicht so locker",
            "more formal",
            "too casual",
            "less casual",
            "more professional",
        ],
        "less formal",
    ),
    (
        "address people with du",
        &["duz*", "per du", "mit du an"],
        "address people with Sie",
    ),
    (
        "address people with Sie",
        &["siez*", "per sie", "mit sie an"],
        "address people with du",
    ),
    (
        "shorter",
        &[
            "kürzer",
            "nicht so lang",
            "zu lang",
            "zu lange",
            "knapper",
            "shorter",
            "too long",
            "more concise",
            "less wordy",
        ],
        "more detailed",
    ),
    (
        "more detailed",
        &[
            "ausführlicher",
            "zu kurz",
            "mehr details",
            "more detail",
            "more details",
            "too short",
        ],
        "shorter",
    ),
    (
        "friendlier",
        &[
            "freundlicher",
            "herzlicher",
            "nicht so kühl",
            "friendlier",
            "warmer",
            "less cold",
        ],
        "",
    ),
];

/// Words that make a recipient or folder a lasting wish.
const LASTING: &[&str] = &[
    "immer",
    "künftig",
    "zukünftig",
    "ab jetzt",
    "ab sofort",
    "in zukunft",
    "standardmäßig",
    "jedes mal",
    "nächstes mal",
    "always",
    "from now on",
    "in future",
    "in the future",
    "by default",
    "every time",
    "next time",
    "going forward",
];

/// Words that end a captured recipient or folder.
const TRAILING: &[&str] = &[
    "ab",
    "ablegen",
    "speichern",
    "sichern",
    "senden",
    "schicken",
    "mailen",
    "weiterleiten",
    "bitte",
    "auch",
    "immer",
    "please",
    "too",
    "also",
    "always",
];

fn patterns() -> &'static [(Kind, Regex)] {
    static PATTERNS: OnceLock<Vec<(Kind, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let who = r"(?P<v>[^,;!?\n]+?)";
        let end = r"(?:\s*$|[,;!?\n]|\.(?:\s|$))";
        let folder = r#"[„"“']?(?P<v>[^,.;!?\n„"“”']+)"#;
        [
            // "immer an Max senden", "schick das künftig immer an max@x.de"
            (Kind::Recipient, format!(r"(?i)\ban\s+{who}\s+(?:senden|schicken|mailen|weiterleiten)\b")),
            (Kind::Recipient, format!(r"(?i)\b(?:schick|sende|leite|maile)\w*\b[^.!?\n]*?\ban\s+{who}{end}")),
            // "immer Lisa in CC"
            (Kind::Recipient, format!(r"(?i)\b(?:immer|always)\s+{who}\s+in\s+(?:cc|kopie)\b")),
            // "always send it to Max", "send them to max@x.com from now on"
            (Kind::Recipient, format!(r"(?i)\b(?:send|email|mail|forward)\w*\b[^.!?\n]*?\bto\s+{who}(?:\s+(?:from now on|in (?:the )?future|by default|next time|going forward)|{end})")),
            (Kind::Recipient, format!(r"(?i)\bcc\s+{who}{end}")),
            // "im Ordner Belege", "in den Ordner „Rechnungen 2026“"
            (Kind::Folder, format!(r"(?i)\b(?:ordner|folder|directory|verzeichnis)\s+{folder}")),
            // "in ~/Documents/Belege"
            (Kind::Folder, r"(?:^|\s)(?P<v>~?/[^\s,;!?]+)".to_owned()),
        ]
        .into_iter()
        .map(|(kind, pattern)| (kind, Regex::new(&pattern).expect("valid pattern")))
        .collect()
    })
}

/// `text`'s sentences (a period inside an address doesn't end one).
fn sentences(text: &str) -> Vec<&str> {
    static END: OnceLock<Regex> = OnceLock::new();
    END.get_or_init(|| Regex::new(r"[.!?;]\s+|\n").expect("valid pattern"))
        .split(text)
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

/// Whether `lower` holds `phrase` as words; a phrase ending in `*` is a
/// stem ("duz*": "duzen", "duz").
fn has_phrase(lower: &str, phrase: &str) -> bool {
    let (stem, open) = match phrase.strip_suffix('*') {
        Some(stem) => (stem, true),
        None => (phrase, false),
    };
    lower.match_indices(stem).any(|(at, _)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + stem.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric())
            && (open || after.is_none_or(|c| !c.is_alphanumeric()))
    })
}

/// A captured value without the words after it ("Max from now on" → "Max").
fn tidy(value: &str) -> Option<String> {
    let mut words: Vec<&str> = value.split_whitespace().collect();
    for phrase in LASTING {
        let lower = words.join(" ").to_lowercase();
        if let Some(at) = lower.find(phrase).filter(|at| *at > 0) {
            let kept = lower[..at].split_whitespace().count();
            words.truncate(kept);
        }
    }
    while words
        .last()
        .is_some_and(|word| TRAILING.contains(&word.to_lowercase().as_str()))
    {
        words.pop();
    }
    let value = words
        .join(" ")
        .trim_matches(|c: char| "„\"“”'".contains(c) || c.is_whitespace())
        .to_owned();
    // A recipient or folder is a name, not a sentence.
    (!value.is_empty() && value.chars().count() <= 80 && words.len() <= 6).then_some(value)
}

/// The preferences in the person's `text` (German or English).
pub fn extract(text: &str) -> Vec<Preference> {
    let mut found: Vec<Preference> = Vec::new();
    // Words about a secret teach nothing (the secret could be in them).
    if crate::assistant::memory::looks_secret(text) {
        return found;
    }
    let said = crate::assistant::store::shorten(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        80,
    );
    let lower = text.to_lowercase();
    // A tone: the first that the words ask for.
    if let Some((tone, _, _)) = TONES
        .iter()
        .find(|(_, words, _)| words.iter().any(|word| has_phrase(&lower, word)))
    {
        // "förmlicher" holds "förmlich": a less-formal phrase wins first,
        // since TONES lists it first.
        found.push(Preference {
            kind: Kind::Tone,
            value: (*tone).to_owned(),
            said: said.clone(),
        });
    }
    for sentence in sentences(text) {
        let lower = sentence.to_lowercase();
        if !LASTING.iter().any(|word| has_phrase(&lower, word)) {
            continue;
        }
        for (kind, pattern) in patterns() {
            if found.iter().any(|known| known.kind == *kind) {
                continue;
            }
            let Some(value) = pattern
                .captures(sentence)
                .and_then(|captures| captures.name("v").map(|v| v.as_str().to_owned()))
                .and_then(|value| tidy(&value))
            else {
                continue;
            };
            if crate::assistant::memory::looks_secret(&value) {
                continue;
            }
            found.push(Preference {
                kind: *kind,
                value,
                said: said.clone(),
            });
        }
    }
    found.retain(|preference| !crate::assistant::memory::looks_secret(&preference.value));
    found
}

/// The start of `preference`'s memory line, by which it is found again.
fn head(kind: Kind, value: &str, project: Option<&str>) -> String {
    match project {
        Some(project) => format!("Preference ({}, project {project}): {value}", kind.word()),
        None => format!("Preference ({}): {value}", kind.word()),
    }
}

/// `note` (the memory note) with `found` added, each once, a tone replacing
/// the one it contradicts; the note and the lines added.
pub fn learn_into(
    note: &str,
    found: &[Preference],
    project: Option<&str>,
) -> (String, Vec<String>) {
    let mut lines: Vec<String> = note.lines().map(str::to_owned).collect();
    let mut added = Vec::new();
    for preference in found {
        let head = head(preference.kind, &preference.value, project);
        let line = format!(
            "- {head} — learned from “{}”",
            preference.said.replace(['“', '”'], "\"")
        );
        if crate::assistant::memory::looks_secret(&line) {
            continue;
        }
        if lines.iter().any(|existing| {
            existing
                .trim_start_matches("- ")
                .to_lowercase()
                .starts_with(&head.to_lowercase())
        }) {
            continue;
        }
        if preference.kind == Kind::Tone
            && let Some((_, _, contradicts)) =
                TONES.iter().find(|(tone, _, _)| *tone == preference.value)
            && !contradicts.is_empty()
        {
            let old = self::head(Kind::Tone, contradicts, project).to_lowercase();
            lines.retain(|existing| {
                !existing
                    .trim_start_matches("- ")
                    .to_lowercase()
                    .starts_with(&old)
            });
        }
        lines.push(line.clone());
        added.push(line);
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    (text, added)
}

/// Learns the preferences in the person's own `text` into the memory note;
/// the lines added. Never in tests, which don't touch the person's note.
pub fn learn(text: &str, project: Option<&str>) -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    let found = extract(text);
    if found.is_empty() {
        return Vec::new();
    }
    let (note, added) = learn_into(&crate::assistant::memory::read(), &found, project);
    if !added.is_empty() {
        if let Err(error) = crate::assistant::memory::write(&note) {
            tracing::warn!(%error, "couldn't keep a learned preference");
            return Vec::new();
        }
        tracing::info!(count = added.len(), "learned the person's preferences");
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(Kind, String)> {
        extract(text)
            .into_iter()
            .map(|preference| (preference.kind, preference.value))
            .collect()
    }

    #[test]
    fn tone_corrections_are_found_in_german_and_english() {
        assert_eq!(
            kinds("Nicht so förmlich bitte"),
            [(Kind::Tone, "less formal".into())]
        );
        assert_eq!(
            kinds("Das ist zu förmlich, schreib lockerer"),
            [(Kind::Tone, "less formal".into())]
        );
        assert_eq!(
            kinds("Bitte etwas förmlicher"),
            [(Kind::Tone, "more formal".into())]
        );
        assert_eq!(
            kinds("too formal, make it more casual"),
            [(Kind::Tone, "less formal".into())]
        );
        assert_eq!(kinds("Kürzer!"), [(Kind::Tone, "shorter".into())]);
        assert_eq!(
            kinds("Schreib ihn per Du an"),
            [(Kind::Tone, "address people with du".into())]
        );
        assert!(kinds("Wie wird das Wetter morgen?").is_empty());
        // Words inside other words don't count.
        assert!(kinds("Das dauert zu langsam, das Dutzend fehlt").is_empty());
        assert!(kinds("I no longer need the file").is_empty());
    }

    #[test]
    fn lasting_recipients_and_folders_are_found_one_offs_are_not() {
        assert_eq!(
            kinds("Schick das künftig immer an max@firma.de"),
            [(Kind::Recipient, "max@firma.de".into())]
        );
        assert_eq!(
            kinds("Die Berichte immer an Lisa Weber senden."),
            [(Kind::Recipient, "Lisa Weber".into())]
        );
        assert_eq!(
            kinds("Always send the invoices to accounting@example.com from now on"),
            [(Kind::Recipient, "accounting@example.com".into())]
        );
        assert_eq!(
            kinds("Rechnungen ab jetzt immer im Ordner „Belege 2026“ ablegen"),
            [(Kind::Folder, "Belege 2026".into())]
        );
        assert_eq!(
            kinds("By default save exports in the folder Reports"),
            [(Kind::Folder, "Reports".into())]
        );
        assert_eq!(
            kinds("Speicher das künftig in ~/Documents/Belege"),
            [(Kind::Folder, "~/Documents/Belege".into())]
        );
        // Said once, for this task only: nothing to keep.
        assert!(kinds("Schick das an Max").is_empty());
        assert!(kinds("Leg es in den Ordner Belege").is_empty());
        assert_eq!(
            kinds("Setz immer Lisa in CC"),
            [(Kind::Recipient, "Lisa".into())]
        );
        assert!(
            kinds("always send an email to bob@example.com")
                .iter()
                .all(|(_, value)| value == "bob@example.com")
        );
        // A correction can carry a tone and a recipient at once.
        assert_eq!(
            kinds("Nicht so förmlich, und schick es immer an Max."),
            [
                (Kind::Tone, "less formal".into()),
                (Kind::Recipient, "Max".into())
            ]
        );
    }

    #[test]
    fn learned_lines_are_reviewable_kept_once_and_a_newer_tone_wins() {
        let note = "- Website: example.com\n";
        let (note, added) = learn_into(note, &extract("nicht so förmlich"), None);
        assert_eq!(
            added,
            ["- Preference (tone): less formal — learned from “nicht so förmlich”"]
        );
        assert!(note.starts_with("- Website: example.com\n- Preference (tone): less formal"));
        // Said again: kept once.
        let (again, added) = learn_into(&note, &extract("Weniger förmlich!"), None);
        assert!(added.is_empty());
        assert_eq!(again, note);
        // The other way now: the newer tone replaces the old one.
        let (note, _) = learn_into(&note, &extract("doch förmlicher bitte"), None);
        assert!(note.contains("Preference (tone): more formal"), "{note}");
        assert!(!note.contains("less formal"), "{note}");
        assert!(note.contains("Website: example.com"));
        // A project's preference names its project.
        let (note, added) = learn_into(
            &note,
            &extract("immer an team@kaliber.de senden"),
            Some("Kalibermarkt"),
        );
        assert_eq!(added.len(), 1);
        assert!(
            note.contains("Preference (recipient, project Kalibermarkt): team@kaliber.de"),
            "{note}"
        );
    }

    #[test]
    fn nothing_secret_is_learned() {
        assert!(extract("Schick das Passwort immer an max@firma.de").is_empty());
        assert!(extract("always send the API key to bob@example.com").is_empty());
        assert!(extract("Speicher das immer im Ordner token-sk-abc").is_empty());
        let secret = Preference {
            kind: Kind::Recipient,
            value: "max".into(),
            said: "mein Passwort ist hunter2".into(),
        };
        let (note, added) = learn_into("", &[secret], None);
        assert!(added.is_empty() && note.is_empty());
    }
}
