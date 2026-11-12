//! How freely tasks may take consequential actions: sending, deleting,
//! paying, sharing outside, publishing. The person picks a mode
//! (`assistant.autonomy`); the guard (desktop) and the browser tool enforce
//! it on the element an action targets, so text a task reads on a web page
//! or in an email can never authorize anything.
//!
//! - **Ask**: every consequential action waits for the person's yes
//!   (`Event::Question`, answered by holding the Action shortcut or in Chats).
//! - **Smart** (default): allowed only when the person's own request named
//!   that kind of action ("send the emails", "share it with Max");
//!   everything else is refused until the person confirms.
//! - **Yolo**: no limits beyond drafts-by-default in the prompts.
//!
//! Per task, what the person authorized lives in a small file under
//! `<support>/assistant/authorized/<task>.json` that the brain writes (from
//! the request and from confirmations) and the proxies read.
//!
//! OWNER: lead (contract); brain (authorization flow); guard and browser
//! (enforcement).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The person's choice in Settings and onboarding.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    Ask,
    #[default]
    Smart,
    Yolo,
}

/// A kind of consequential action.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Sending a message, email, or invitation.
    Send,
    /// Deleting, emptying trash, removing accounts or files outside the
    /// task's workspace.
    Delete,
    /// Buying, paying, subscribing, transferring money.
    Pay,
    /// Sharing with or inviting someone, changing who can see something.
    Share,
    /// Posting or publishing publicly, submitting forms to third parties.
    Publish,
}

impl Category {
    pub const ALL: [Self; 5] = [
        Self::Send,
        Self::Delete,
        Self::Pay,
        Self::Share,
        Self::Publish,
    ];
}

/// What a task may do without asking, beyond the mode's defaults.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Authorized {
    pub categories: BTreeSet<Category>,
    /// Free-text limits the person gave ("only to max@example.com").
    pub notes: Vec<String>,
}

/// The kind of consequential action a control's label means, if any:
/// "Send", "Senden", "Delete", "Löschen", "Buy now", "Jetzt kaufen",
/// "Pay", "Bezahlen", "Share", "Teilen", "Publish", "Veröffentlichen", …
/// (English and German at least; the enforcement owners extend it).
///
/// A label that ends in an ellipsis ("Share…", "Delete…") only opens a
/// dialog or menu, so it is none: the dialog's own button is classified
/// when it is pressed.
pub fn classify(label: &str) -> Option<Category> {
    let trimmed = label.trim();
    if trimmed.ends_with('…') || trimmed.ends_with("...") {
        return None;
    }
    let words = normalize(trimmed);
    if words.is_empty()
        || NOT_CONSEQUENTIAL
            .iter()
            .any(|phrase| starts_with(&words, phrase))
    {
        return None;
    }
    KEYWORDS
        .iter()
        .find(|(phrase, _)| starts_with(&words, phrase))
        .map(|(_, category)| *category)
}

/// Lowercase words, without punctuation and price tags.
fn normalize(label: &str) -> String {
    label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `words` is `phrase`, or begins with it as whole words.
fn starts_with(words: &str, phrase: &str) -> bool {
    words == phrase
        || words
            .strip_prefix(phrase)
            .is_some_and(|rest| rest.starts_with(' '))
}

/// Labels that begin like a consequential one but aren't.
const NOT_CONSEQUENTIAL: &[&str] = &[
    "send to back",
    "send backward",
    "nach hinten",
    "share screen",
    "delete backward",
    "delete forward",
    "post it",
];

/// Leading words and what they mean, English and German.
const KEYWORDS: &[(&str, Category)] = &[
    ("send", Category::Send),
    ("send now", Category::Send),
    ("send message", Category::Send),
    ("send email", Category::Send),
    ("send invitation", Category::Send),
    ("senden", Category::Send),
    ("absenden", Category::Send),
    ("jetzt senden", Category::Send),
    ("nachricht senden", Category::Send),
    ("verschicken", Category::Send),
    ("delete", Category::Delete),
    ("löschen", Category::Delete),
    ("endgültig löschen", Category::Delete),
    ("move to trash", Category::Delete),
    ("move to bin", Category::Delete),
    ("in den papierkorb", Category::Delete),
    ("empty trash", Category::Delete),
    ("empty bin", Category::Delete),
    ("papierkorb entleeren", Category::Delete),
    ("erase", Category::Delete),
    ("remove", Category::Delete),
    ("entfernen", Category::Delete),
    ("delete account", Category::Delete),
    ("konto löschen", Category::Delete),
    ("buy", Category::Pay),
    ("buy now", Category::Pay),
    ("kaufen", Category::Pay),
    ("jetzt kaufen", Category::Pay),
    ("pay", Category::Pay),
    ("pay now", Category::Pay),
    ("bezahlen", Category::Pay),
    ("jetzt bezahlen", Category::Pay),
    ("zahlungspflichtig bestellen", Category::Pay),
    ("kostenpflichtig bestellen", Category::Pay),
    ("place order", Category::Pay),
    ("place your order", Category::Pay),
    ("bestellung aufgeben", Category::Pay),
    ("purchase", Category::Pay),
    ("checkout", Category::Pay),
    ("subscribe", Category::Pay),
    ("abonnieren", Category::Pay),
    ("transfer", Category::Pay),
    ("überweisen", Category::Pay),
    ("share", Category::Share),
    ("teilen", Category::Share),
    ("freigeben", Category::Share),
    ("invite", Category::Share),
    ("einladen", Category::Share),
    ("add people", Category::Share),
    ("personen hinzufügen", Category::Share),
    ("publish", Category::Publish),
    ("veröffentlichen", Category::Publish),
    ("post", Category::Publish),
    ("posten", Category::Publish),
    ("tweet", Category::Publish),
    ("submit", Category::Publish),
    ("einreichen", Category::Publish),
];

/// Whether an action of `category` may run now for this task.
pub fn allowed(mode: Autonomy, category: Category, authorized: &Authorized) -> bool {
    match mode {
        Autonomy::Yolo => true,
        Autonomy::Smart => authorized.categories.contains(&category),
        Autonomy::Ask => false,
    }
}

/// The task's authorization file, read by the proxies.
pub fn read(task: &str) -> Authorized {
    let _ = task;
    Authorized::default()
}

/// Writes it (the brain, from the request and from confirmations).
pub fn write(task: &str, authorized: &Authorized) {
    let _ = (task, authorized);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consequential_labels_are_classified_in_english_and_german() {
        for (label, category) in [
            ("Send", Category::Send),
            ("Senden", Category::Send),
            ("send message", Category::Send),
            ("Move to Trash", Category::Delete),
            ("In den Papierkorb legen", Category::Delete),
            ("Empty Trash", Category::Delete),
            ("Papierkorb entleeren", Category::Delete),
            ("Delete", Category::Delete),
            ("Löschen", Category::Delete),
            ("Buy Now", Category::Pay),
            ("Jetzt kaufen", Category::Pay),
            ("Zahlungspflichtig bestellen", Category::Pay),
            ("Place your order", Category::Pay),
            ("Share", Category::Share),
            ("Freigeben", Category::Share),
            ("Publish", Category::Publish),
            ("Veröffentlichen", Category::Publish),
            ("Post", Category::Publish),
        ] {
            assert_eq!(classify(label), Some(category), "{label}");
        }
    }

    #[test]
    fn dialogs_and_lookalikes_are_not_consequential() {
        for label in [
            "Share…",
            "Delete...",
            "Send to Back",
            "Sender",
            "Sendung verfolgen",
            "Deleted Items",
            "Postfach",
            "Buyer guide",
            "Reply",
            "Cancel",
            "",
        ] {
            assert_eq!(classify(label), None, "{label}");
        }
    }
}
