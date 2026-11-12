//! A short note about the person that every task reads (when
//! `assistant.memory` is on): their business, website, the people they work
//! with, the tone they like. It lives at `<support>/assistant/memory.md`,
//! readable only by this user; the person edits it in Settings ([`read`],
//! [`write`]) and tasks add to it with `hex_remember` ([`remember`]), one
//! line per fact. Secrets never go in.
//!
//! OWNER: brain.

use std::path::PathBuf;
use std::sync::OnceLock;

use color_eyre::{Result, eyre::eyre};
use regex::Regex;

/// How much of the note a prompt carries: about 1,000 tokens.
const PROMPT_CHARS: usize = 4000;

fn path() -> Result<PathBuf> {
    Ok(crate::assistant::store::assistant_dir()?.join("memory.md"))
}

/// The note as it is; empty when there is none.
pub fn read() -> String {
    path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Replaces the note (Settings' editor).
pub fn write(text: &str) -> Result<()> {
    let path = path()?;
    crate::assistant::store::write_private(&path, text.as_bytes())
        .map_err(|error| eyre!("couldn't save the assistant's memory: {error}"))
}

/// Adds `fact` as a line, unless it is already there or looks like a
/// secret; the line added.
pub fn remember(fact: &str) -> Result<String> {
    let line = clean(fact)?;
    let mut note = read();
    if note.lines().any(|existing| {
        existing
            .trim_start_matches("- ")
            .eq_ignore_ascii_case(&line)
    }) {
        return Ok(line);
    }
    if !note.is_empty() && !note.ends_with('\n') {
        note.push('\n');
    }
    note.push_str(&format!("- {line}\n"));
    write(&note)?;
    Ok(line)
}

/// `fact` on one line, or why it can't be kept.
fn clean(fact: &str) -> Result<String> {
    let line = fact.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return Err(eyre!("nothing to remember"));
    }
    if line.chars().count() > 300 {
        return Err(eyre!("too long to remember: keep it to one short fact"));
    }
    if looks_secret(&line) {
        return Err(eyre!(
            "that looks like a secret (a password, key, or card number); Hex never remembers those"
        ));
    }
    Ok(line)
}

/// Whether `text` holds what must never be remembered: passwords, keys,
/// tokens, card or account numbers.
pub fn looks_secret(text: &str) -> bool {
    static SECRET: OnceLock<Regex> = OnceLock::new();
    SECRET
        .get_or_init(|| {
            Regex::new(
                r"(?i)password|passwort|kennwort|passcode|\bpin\b|api[ _-]?key|secret|token|private key|\bsk-[a-z0-9]|\bghp_|\bxox[bp]-|\b(?:\d[ -]?){13,19}\b|\biban\b|\bcvv\b|\bcvc\b",
            )
            .expect("valid pattern")
        })
        .is_match(text)
}

/// The note for a prompt: its newest part when it is long.
pub fn for_prompt(note: &str) -> Option<String> {
    let note = note.trim();
    if note.is_empty() {
        return None;
    }
    let count = note.chars().count();
    if count <= PROMPT_CHARS {
        return Some(note.to_owned());
    }
    let tail: String = note.chars().skip(count - PROMPT_CHARS).collect();
    let tail = tail
        .split_once('\n')
        .map_or(tail.as_str(), |(_, rest)| rest);
    Some(format!("(older notes left out)\n{tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_never_remembered() {
        assert_eq!(
            clean("  The colleague is Max,\n max@example.com ").unwrap(),
            "The colleague is Max, max@example.com"
        );
        for secret in [
            "My Gmail password is hunter2",
            "API key sk-abc123",
            "Card 4111 1111 1111 1111",
            "Das Passwort ist geheim",
            "IBAN DE89 3704 0044 0532 0130 00",
        ] {
            assert!(clean(secret).is_err(), "{secret}");
        }
        assert!(clean("").is_err());
        assert!(!looks_secret("Website: blabla.com, tone: friendly, du"));
    }

    #[test]
    fn a_long_note_is_cut_to_its_newest_lines() {
        assert_eq!(for_prompt("  \n"), None);
        assert_eq!(for_prompt("- a").as_deref(), Some("- a"));
        let long: String = (0..400).map(|i| format!("- fact number {i}\n")).collect();
        let cut = for_prompt(&long).unwrap();
        assert!(cut.starts_with("(older notes left out)\n- fact number"));
        assert!(cut.ends_with("- fact number 399"));
        assert!(cut.chars().count() <= PROMPT_CHARS + 30);
    }
}
