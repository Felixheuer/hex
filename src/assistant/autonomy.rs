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
/// in English, German, French, and Spanish. A label that undoes, cancels,
/// or discards a draft is none ("Discard draft", "Entwurf verwerfen");
/// keyboard hints such as the "(⌘Enter)" in Gmail's "Send (⌘Enter)" are ignored.
///
/// A label that ends in an ellipsis ("Share…", "Delete…") only opens a
/// dialog or menu, so it is none: the dialog's own button is classified
/// when it is pressed. Desktop commands that merely begin like one ("Send
/// to Back", "Share Screen", "Delete Backward") are none too.
pub fn classify(label: &str) -> Option<Category> {
    let trimmed = label.trim();
    if trimmed.ends_with('…') || trimmed.ends_with("...") {
        return None;
    }
    let flat = normalize(trimmed);
    if NOT_CONSEQUENTIAL
        .iter()
        .any(|phrase| starts_with(&flat, phrase))
    {
        return None;
    }
    let words = words(label);
    if words.is_empty() || SAFE.iter().any(|phrase| has(&words, phrase)) {
        return None;
    }
    // Most specific first: "Send money" pays, "Submit order" pays.
    [
        (Category::Pay, PAY),
        (Category::Delete, DELETE),
        (Category::Share, SHARE),
        (Category::Send, SEND),
        (Category::Publish, PUBLISH),
    ]
    .into_iter()
    .find(|(_, phrases)| phrases.iter().any(|phrase| has(&words, phrase)))
    .map(|(category, _)| category)
}

/// Undoing, cancelling, closing, or throwing away a draft.
const SAFE: &[&str] = &[
    "discard",
    "discard draft",
    "cancel",
    "undo",
    "close",
    "verwerfen",
    "abbrechen",
    "rückgängig",
    "schließen",
    "annuler",
    "fermer",
    "ignorer",
    "cancelar",
    "deshacer",
    "descartar",
    "cerrar",
];

const PAY: &[&str] = &[
    "buy",
    "buy now",
    "purchase",
    "pay",
    "pay now",
    "place order",
    "place your order",
    "submit order",
    "complete purchase",
    "confirm purchase",
    "subscribe",
    "transfer",
    "send money",
    "kaufen",
    "jetzt kaufen",
    "kaufen und bezahlen",
    "zahlungspflichtig bestellen",
    "kostenpflichtig bestellen",
    "bestellen",
    "bezahlen",
    "zahlen",
    "abonnieren",
    "überweisen",
    "acheter",
    "payer",
    "commander",
    "s'abonner",
    "abonner",
    "virement",
    "comprar",
    "pagar",
    "realizar pedido",
    "suscribirse",
    "suscribir",
    "transferir",
];

const DELETE: &[&str] = &[
    "delete",
    "delete forever",
    "delete permanently",
    "remove",
    "trash",
    "move to trash",
    "move to bin",
    "empty trash",
    "empty bin",
    "erase",
    "löschen",
    "endgültig löschen",
    "entfernen",
    "papierkorb leeren",
    "papierkorb entleeren",
    "in den papierkorb",
    "supprimer",
    "supprimer définitivement",
    "vider la corbeille",
    "mettre à la corbeille",
    "eliminar",
    "borrar",
    "vaciar papelera",
    "mover a la papelera",
];

const SHARE: &[&str] = &[
    "share",
    "invite",
    "give access",
    "make public",
    "freigeben",
    "teilen",
    "einladen",
    "partager",
    "inviter",
    "compartir",
    "invitar",
];

const SEND: &[&str] = &[
    "send",
    "send now",
    "reply and send",
    "senden",
    "sende",
    "absenden",
    "verschicken",
    "versenden",
    "envoyer",
    "enviar",
];

const PUBLISH: &[&str] = &[
    "post",
    "publish",
    "tweet",
    "submit",
    "veröffentlichen",
    "posten",
    "einreichen",
    "publier",
    "poster",
    "soumettre",
    "publicar",
    "someter",
];

/// A label's lowercase words, without keyboard hints in parentheses and
/// the bidi marks Gmail puts around them.
fn words(label: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0_u32;
    for c in label.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() || c == '\'' => plain.extend(c.to_lowercase()),
            _ => plain.push(' '),
        }
    }
    plain.split_whitespace().map(str::to_owned).collect()
}

/// Whether `phrase`'s words appear in `words`, in order and together.
fn has(words: &[String], phrase: &str) -> bool {
    let phrase: Vec<&str> = phrase.split(' ').collect();
    words
        .windows(phrase.len())
        .any(|window| window.iter().zip(&phrase).all(|(word, part)| word == part))
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


/// Whether an action of `category` may run now for this task.
pub fn allowed(mode: Autonomy, category: Category, authorized: &Authorized) -> bool {
    match mode {
        Autonomy::Yolo => true,
        Autonomy::Smart => authorized.categories.contains(&category),
        Autonomy::Ask => false,
    }
}

/// `<support>/assistant/authorized/<task>.json`; `None` for an id that
/// isn't a plain name.
fn path(task: &str) -> Option<std::path::PathBuf> {
    let plain = !task.is_empty()
        && task.len() <= 128
        && task
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
        return None;
    }
    let dir = crate::assistant::store::assistant_dir()
        .ok()?
        .join("authorized");
    Some(dir.join(format!("{task}.json")))
}

/// The task's authorization file, read by the proxies; nothing authorized
/// when there is none (or it can't be read).
pub fn read(task: &str) -> Authorized {
    path(task)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes it (the brain, from the request and from confirmations),
/// private to this user, replacing it whole.
pub fn write(task: &str, authorized: &Authorized) {
    let Some(path) = path(task) else {
        tracing::warn!(task, "no authorization file for this task id");
        return;
    };
    let written = (|| -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(&path);
        crate::app_paths::create_private_dir(dir)?;
        let temporary = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(
            &mut options.open(&temporary)?,
            &serde_json::to_vec(authorized).unwrap_or_default(),
        )?;
        std::fs::rename(&temporary, &path)
    })();
    if let Err(error) = written {
        tracing::warn!(%error, task, "couldn't write the task's authorization");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_labels_are_classified_in_four_languages() {
        for (label, category) in [
            ("Send", Category::Send),
            ("Send \u{202a}(⌘Enter)\u{202c}", Category::Send),
            ("Senden", Category::Send),
            ("Envoyer", Category::Send),
            ("Enviar", Category::Send),
            ("Delete", Category::Delete),
            ("Löschen", Category::Delete),
            ("Endgültig löschen", Category::Delete),
            ("Supprimer", Category::Delete),
            ("Eliminar", Category::Delete),
            ("Empty Trash now", Category::Delete),
        ] {
            assert_eq!(classify(label), Some(category), "{label}");
        }
        for label in [
            "Discard draft \u{202a}(⌘⇧D)\u{202c}",
            "Entwurf verwerfen",
            "Annuler l'envoi",
            "Undo send",
            "Cancelar",
            "Archive",
            "Reply",
            "Sent",
            "Search",
            "Copy link",
            "Done",
            "",
        ] {
            assert_eq!(classify(label), None, "{label}");
        }
    }

    #[test]
    fn shop_share_and_post_labels_are_classified() {
        for (label, category) in [
            ("Buy now", Category::Pay),
            ("Jetzt kaufen", Category::Pay),
            ("Bezahlen", Category::Pay),
            ("Zahlungspflichtig bestellen", Category::Pay),
            ("Submit order", Category::Pay),
            ("Place your order", Category::Pay),
            ("Acheter", Category::Pay),
            ("Comprar ahora", Category::Pay),
            ("Send money", Category::Pay),
            ("Share", Category::Share),
            ("Freigeben", Category::Share),
            ("Teilen", Category::Share),
            ("Partager", Category::Share),
            ("Compartir", Category::Share),
            ("Invite people", Category::Share),
            ("Post", Category::Publish),
            ("Veröffentlichen", Category::Publish),
            ("Publier", Category::Publish),
            ("Publicar", Category::Publish),
            ("Submit", Category::Publish),
        ] {
            assert_eq!(classify(label), Some(category), "{label}");
        }
        for label in [
            "Add to cart",
            "Proceed to checkout",
            "Posts",
            "Shared with me",
        ] {
            assert_eq!(classify(label), None, "{label}");
        }
    }

    #[test]
    fn each_mode_allows_what_it_promises() {
        let mut authorized = Authorized::default();
        assert!(allowed(Autonomy::Yolo, Category::Pay, &authorized));
        assert!(!allowed(Autonomy::Smart, Category::Send, &authorized));
        authorized.categories.insert(Category::Send);
        assert!(allowed(Autonomy::Smart, Category::Send, &authorized));
        assert!(!allowed(Autonomy::Smart, Category::Delete, &authorized));
        assert!(!allowed(Autonomy::Ask, Category::Send, &authorized));
    }

    #[test]
    fn only_plain_task_ids_have_an_authorization_file() {
        assert!(path("../escape").is_none());
        assert!(path("a/b").is_none());
        assert!(path("").is_none());
        let json = r#"{"categories":["send","share"],"notes":["only to max@example.com"]}"#;
        let authorized: Authorized = serde_json::from_str(json).unwrap();
        assert!(authorized.categories.contains(&Category::Share));
        assert_eq!(serde_json::to_string(&authorized).unwrap(), json);
    }
}

#[cfg(test)]
mod desktop_tests {
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
