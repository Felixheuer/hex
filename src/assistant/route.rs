//! Every word-list heuristic that steers a request, in one place: the
//! spoken opening that sends a hold to the Background lane
//! ([`spoken_background`]), words asking a task to stay out of the person's
//! windows ([`asks_background_only`]), an answer that claims it started the
//! work in the background without a job ([`claims_background`]), and words
//! asking for the web, which start a hold's browser early ([`wants_web`]).
//!
//! These are cheap guesses, not the routing: the Here agent decides what is
//! a job and hands it on with `hex_start_jobs` (`agent::routing`). Each list
//! carries the languages Hex speaks most (German, English, French, Spanish,
//! Italian, Dutch); `route/tests.rs` holds one table per language.
//!
//! OWNER: brain.

/// Spoken openings that send a question to the Background lane, as words
/// compared without case or the punctuation around them. `true`: only with a
/// colon after it ("Hintergrund: …"), since "Hintergrund ändern" asks about
/// the background itself.
const SPOKEN_BACKGROUND: &[(&[&str], bool)] = &[
    (&["im", "hintergrund"], false),
    (&["in", "the", "background"], false),
    (&["nebenbei"], false),
    (&["parallel"], false),
    (&["hintergrund"], true),
    (&["en", "arrière-plan"], false),
    (&["en", "segundo", "plano"], false),
    (&["in", "background"], false),
    (&["op", "de", "achtergrond"], false),
];

/// `text` without its spoken opening when it asks for the Background lane
/// ("Im Hintergrund, räum den Downloads-Ordner auf" becomes "räum den
/// Downloads-Ordner auf"); `None` when it doesn't start with one. A word
/// that merely starts like one ("Parallelogramm", "Hintergrundbild") is not
/// one.
pub(crate) fn spoken_background(text: &str) -> Option<&str> {
    let text = text.trim();
    SPOKEN_BACKGROUND.iter().find_map(|&(words, colon)| {
        let mut rest = text;
        let mut last = "";
        for &word in words {
            let trimmed = rest.trim_start();
            let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
            let (spoken, after) = trimmed.split_at(end);
            let bare = spoken.trim_matches(|c: char| !c.is_alphanumeric());
            if bare.to_lowercase() != word {
                return None;
            }
            last = spoken;
            rest = after;
        }
        // "Hintergrund:" or "Hintergrund :".
        if colon && !last.ends_with(':') {
            rest = rest.trim_start().strip_prefix(':')?;
        }
        Some(
            rest.trim_start_matches(|c: char| !c.is_alphanumeric())
                .trim_end(),
        )
    })
}

/// Phrases asking a task to keep out of the person's windows.
const BACKGROUND_ONLY: &[&str] = &[
    // German
    "nur im hintergrund",
    "ausschließlich im hintergrund",
    "lass meine fenster",
    "nicht in meinen fenstern",
    "nicht in meinem fenster",
    "fass meine fenster nicht",
    // English
    "only in the background",
    "don't touch my windows",
    "leave my windows",
    // French
    "seulement en arrière-plan",
    "uniquement en arrière-plan",
    "ne touche pas à mes fenêtres",
    // Spanish
    "solo en segundo plano",
    "no toques mis ventanas",
    // Italian
    "solo in background",
    "non toccare le mie finestre",
    // Dutch
    "alleen op de achtergrond",
    "laat mijn vensters",
];

/// Whether `request` asks the task to stay in the background, out of the
/// person's windows („arbeite nur im Hintergrund“, "only in the
/// background"): it then gets no full control (`assistant::full_control`).
pub fn asks_background_only(request: &str) -> bool {
    let text = request.to_lowercase().replace('’', "'");
    BACKGROUND_ONLY.iter().any(|phrase| text.contains(phrase))
}

/// Phrases with which an answer says it *starts* the work in the
/// background now (not that something already runs there).
const CLAIMS: &[&str] = &[
    // German
    "läuft als hintergrundauftrag",
    "läuft als hintergrundaufgabe",
    "als hintergrundauftrag",
    "als hintergrundaufgabe",
    "mach ich im hintergrund",
    "mache ich im hintergrund",
    "erledige ich im hintergrund",
    "übernehme ich im hintergrund",
    "kümmere mich im hintergrund",
    "starte ich im hintergrund",
    "ich starte im hintergrund",
    "schiebe ich in den hintergrund",
    "in den hintergrund verschoben",
    // English
    "i'll do that in the background",
    "i'll do it in the background",
    "i'll handle that in the background",
    "i'll handle it in the background",
    "i'm doing it in the background",
    "as a background job",
    "as a background task",
    "moved it to the background",
    "running it in the background",
    // French
    "je le fais en arrière-plan",
    "je m'en occupe en arrière-plan",
    "comme tâche de fond",
    // Spanish
    "lo hago en segundo plano",
    "me encargo en segundo plano",
    // Italian
    "lo faccio in background",
    "me ne occupo in background",
    // Dutch
    "ik doe het op de achtergrond",
    "ik regel het op de achtergrond",
];

/// Phrases that say the opposite (it can't, or the person should).
const DENIALS: &[&str] = &[
    "kann ich nicht",
    "nicht möglich",
    "nicht im hintergrund",
    "can't",
    "cannot",
    "not in the background",
    "je ne peux pas",
    "no puedo",
    "non posso",
    "kan ik niet",
];

/// Whether an answer claims it starts the request in the background now,
/// though no job was started: the safety net under the tool and the marker.
pub fn claims_background(answer: &str) -> bool {
    let answer = answer.to_lowercase().replace('’', "'");
    CLAIMS.iter().any(|claim| answer.contains(claim))
        && !DENIALS.iter().any(|denial| answer.contains(denial))
}

/// Words that ask for the web: a browser, a site, a search.
const WEB: &[&str] = &[
    // German
    "browser",
    "webseite",
    "im web",
    "suche nach",
    "such nach",
    "such mal",
    "nachschauen",
    "googel",
    // English
    "website",
    "web site",
    "internet",
    "on the web",
    "online",
    "search for",
    "look up",
    // French
    "navigateur",
    "site web",
    "cherche sur",
    "recherche sur",
    // Spanish
    "navegador",
    "sitio web",
    "busca en",
    // Italian
    "sito web",
    "cerca su",
    // Dutch
    "zoek op",
    "zoek naar",
];

/// Whether words said so far ask for the web: a browser or a search, an
/// address (written, or spoken as "spiegel punkt de"), or one of the
/// `signed_in` sites, those the person signed in to for Hex
/// (`browser::signed_in_sites`). No list of sites.
pub fn wants_web(words: &str, signed_in: &[String]) -> bool {
    let text = words.to_lowercase();
    WEB.iter().any(|word| text.contains(word))
        || names_address(&text)
        || crate::assistant::browser::names_site(&text, signed_in)
}

/// Whether `text` (lowercase) holds a web address: `https://…`, `www.…`, a
/// host such as `spiegel.de`, or one spoken (`spiegel punkt de`, `example
/// dot com`).
pub fn names_address(text: &str) -> bool {
    const SPOKEN_DOT: &[&str] = &["punkt", "dot"];
    const SPOKEN_SUFFIX: &[&str] = &[
        "de", "com", "org", "net", "io", "at", "ch", "eu", "info", "ai", "app", "dev", "co", "uk",
        "fr", "nl", "es", "it",
    ];
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| ",;:!?()\"'«»„“”".contains(c)))
        .collect();
    let host = |word: &str| {
        let word = word
            .split('/')
            .next()
            .unwrap_or_default()
            .trim_end_matches('.');
        let Some((name, suffix)) = word.rsplit_once('.') else {
            return false;
        };
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
            && (2..=6).contains(&suffix.len())
            && suffix.chars().all(|c| c.is_ascii_lowercase())
    };
    words.iter().any(|word| {
        word.starts_with("http://") || word.starts_with("https://") || word.starts_with("www.")
    }) || words.iter().any(|word| host(word))
        || words.windows(3).any(|three| {
            !three[0].is_empty()
                && SPOKEN_DOT.contains(&three[1])
                && SPOKEN_SUFFIX.contains(&three[2])
        })
}

#[cfg(test)]
mod tests;
