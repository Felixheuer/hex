//! What the person says to switch projects, answered by Agent-Desk itself
//! without asking an agent: „wechsle zu Projekt Startup“, „neues Projekt
//! Kalibermarkt“, „ohne Projekt“, „welches Projekt?“, and the same in
//! English. Only a whole utterance that is one of these counts: „mach eine
//! Präsentation für das Projekt Startup“ is a request (in which the project
//! may be suggested, [`super::Projects::suggest`]).

use std::sync::OnceLock;

use regex::Regex;

/// A spoken project command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Work in the project with this name from now on.
    Switch(String),
    /// Make a project with this name and work in it.
    Create(String),
    /// Work without a project from now on.
    Leave,
    /// Which project is active?
    Which,
}

/// The most words a project's name may have in a command.
const NAME_WORDS: usize = 4;

/// Makes the command from the name it captured.
type Make = fn(String) -> Command;

fn patterns() -> &'static [(Regex, Make)] {
    static PATTERNS: OnceLock<Vec<(Regex, Make)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let leave = |_| Command::Leave;
        let which = |_| Command::Which;
        let switch: Make = Command::Switch;
        let create: Make = Command::Create;
        let table: [(&str, Make); 16] = [
            // German.
            (r"(?:ohne|kein(?:e)?)\s+projekt(?:\s+mehr)?", leave),
            (
                r"(?:(?:das\s+)?projekt\s+(?:verlassen|schließen|schliessen|beenden)|(?:verlasse|verlass|schließe|schliesse|schließ|beende|beend)\s+(?:das\s+)?projekt|raus\s+aus\s+dem\s+projekt)",
                leave,
            ),
            (
                r"(?:welches|was\s+für\s+ein)\s+projekt(?:\s+ist\s+(?:aktiv|an|gerade\s+aktiv|ausgewählt))?|in\s+welchem\s+projekt\s+(?:bin|sind)\s+(?:ich|wir)(?:\s+gerade)?",
                which,
            ),
            (
                r"(?:(?:erstelle|erstell|lege|leg|mache|mach)\s+(?:mir\s+)?(?:ein\s+)?neues\s+projekt|neues\s+projekt)(?:\s+(?:namens|mit\s+dem\s+namen|für))?\s+(?P<name>.+?)(?:\s+an)?",
                create,
            ),
            (
                r"(?:lege|leg)\s+(?:mir\s+)?(?:ein\s+)?projekt\s+(?:namens\s+)?(?P<name>.+?)\s+an",
                create,
            ),
            (
                r"(?:wechsle|wechsel|wechseln|geh|gehe|spring|springe|schalte|schalt)\s+(?:bitte\s+)?(?:zu|zum|in|ins|auf)\s+(?:das\s+|dem\s+|den\s+)?projekt\s+(?P<name>.+?)(?:\s+um)?",
                switch,
            ),
            (
                r"(?:wechsle|wechsel)\s+(?:das\s+)?projekt\s+(?:zu|auf)\s+(?P<name>.+)",
                switch,
            ),
            (
                r"(?:öffne|oeffne|aktiviere|nimm|nehme)\s+(?:das\s+)?projekt\s+(?P<name>.+)",
                switch,
            ),
            (
                r"projekt\s+(?P<name>.+?)\s+(?:öffnen|oeffnen|aktivieren|bitte)",
                switch,
            ),
            (
                r"(?:arbeite|arbeiten|wir\s+arbeiten)\s+(?:jetzt\s+)?(?:im|in\s+dem|am)\s+projekt\s+(?P<name>.+)",
                switch,
            ),
            // English.
            (
                r"(?:no|without\s+(?:a|any)?)\s*project|(?:leave|close|exit|quit)\s+(?:the\s+)?project",
                leave,
            ),
            (
                r"(?:which|what)\s+project(?:\s+is\s+(?:active|on|open))?|what\s+project\s+am\s+i\s+in",
                which,
            ),
            (
                r"(?:(?:create|make|start|add)\s+(?:a\s+)?new\s+project|(?:create|make|add)\s+a\s+project|new\s+project)(?:\s+(?:called|named|for))?\s+(?P<name>.+)",
                create,
            ),
            (
                r"(?:switch|change|go)\s+(?:over\s+)?to\s+(?:the\s+)?project\s+(?P<name>.+)",
                switch,
            ),
            (
                r"(?:switch|change|go)\s+(?:over\s+)?to\s+(?:the\s+)?(?P<name>.+?)\s+project",
                switch,
            ),
            (
                r"(?:open|use|activate|work\s+(?:in|on))\s+(?:the\s+)?project\s+(?P<name>.+)",
                switch,
            ),
        ];
        table
            .into_iter()
            .map(|(source, make)| {
                (
                    Regex::new(&format!(r"(?i)^(?:{source})$")).expect("valid pattern"),
                    make,
                )
            })
            .collect()
    })
}

/// The command `said` is, if it is one as a whole.
pub fn parse(said: &str) -> Option<Command> {
    let text = clean(said);
    if text.is_empty() {
        return None;
    }
    for (pattern, make) in patterns() {
        let Some(found) = pattern.captures(&text) else {
            continue;
        };
        let name = found
            .name("name")
            .map(|name| name_of(name.as_str()))
            .unwrap_or_default();
        if found.name("name").is_some()
            && (name.is_empty() || name.split_whitespace().count() > NAME_WORDS)
        {
            continue;
        }
        return Some(make(name));
    }
    None
}

/// `said` on one line, without "bitte"/"please" at either end or the
/// punctuation that ends it.
fn clean(said: &str) -> String {
    let mut text = said.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let trimmed = text
            .trim()
            .trim_end_matches(['.', '!', '?', ',', '。', '！', '？'])
            .trim();
        let lower = trimmed.to_lowercase();
        let next = ["bitte ", "please ", "okay ", "ok ", "hey "]
            .iter()
            .find(|word| lower.starts_with(*word))
            .map(|word| trimmed[word.len()..].to_owned())
            .or_else(|| {
                [" bitte", " please"]
                    .iter()
                    .find(|word| lower.ends_with(*word))
                    .map(|word| trimmed[..trimmed.len() - word.len()].to_owned())
            });
        match next {
            Some(next) => text = next.trim_start_matches(',').trim().to_owned(),
            None => return trimmed.to_owned(),
        }
    }
}

/// A spoken name without its quotes and the punctuation around it.
fn name_of(said: &str) -> String {
    said.trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '„' | '“' | '”' | '‚' | '‘' | '’' | '«' | '»' | ',' | ':'
            )
        })
        .trim()
        .to_owned()
}

/// A request scoped to one project for itself alone, „im Projekt Startup:
/// schreib die Mail“ or „in project Startup, write the mail“: the name it
/// names (matched against the projects by the caller, never a list here)
/// and the request. The active project stays as it is.
pub fn scoped(said: &str) -> Option<(String, String)> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"(?is)^\s*(?:im|in\s+dem|in|für\s+das|für|fürs|for\s+the|for|in\s+the|in)\s+(?:projekt|project)\s+(?P<name>[^:,–—]+?)\s*[:,–—]\s*(?P<rest>\S.*)$",
        )
        .expect("valid pattern")
    });
    let found = pattern.captures(said.trim())?;
    let name = name_of(found.name("name")?.as_str());
    let rest = found.name("rest")?.as_str().trim().to_owned();
    (!name.is_empty() && name.split_whitespace().count() <= NAME_WORDS && !rest.is_empty())
        .then_some((name, rest))
}
