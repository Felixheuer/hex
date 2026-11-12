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
pub fn classify(label: &str) -> Option<Category> {
    let _ = label;
    None
}

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
