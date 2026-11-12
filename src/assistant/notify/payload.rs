//! What Windows and Linux are handed for a notification: a toast's XML, and
//! the arguments of `org.freedesktop.Notifications.Notify`. Kept apart from
//! the OS calls so it is tested on every OS.

use super::Note;

/// The app id notifications are shown under: Hex's Linux desktop entry
/// (`anomaly.hex.desktop`) and its Windows AppUserModelID.
pub const APP_ID: &str = "anomaly.hex";

/// The action a click on a freedesktop notification invokes.
#[cfg(any(test, target_os = "linux"))]
pub const DEFAULT_ACTION: &str = "default";

/// `note` as a toast: its title, then its body; a click (anywhere on it)
/// opens the conversation, which `launch` names for the record.
#[cfg(any(test, target_os = "windows"))]
pub fn toast_xml(note: &Note) -> String {
    format!(
        "<toast launch=\"hex://chats?id={}\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml(&note.conversation),
        xml(&note.title),
        xml(&note.body)
    )
}

/// `text` safe inside XML text and attributes.
#[cfg(any(test, target_os = "windows"))]
fn xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            // XML 1.0 allows no other control characters.
            '\t' | '\n' | '\r' => escaped.push(c),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    escaped
}

/// `Notify`'s arguments, but the hints' variant values.
#[cfg(any(test, target_os = "linux"))]
#[derive(Clone, Debug, PartialEq)]
pub struct Notify {
    pub app_name: &'static str,
    pub replaces_id: u32,
    pub app_icon: &'static str,
    pub summary: String,
    pub body: String,
    /// Pairs of action key and label: the default action opens the chat.
    pub actions: Vec<String>,
    /// `desktop-entry`: whose notification it is (and its icon).
    pub desktop_entry: &'static str,
    /// -1: as long as the server keeps notifications.
    pub expire_timeout: i32,
}

/// `note` for `org.freedesktop.Notifications`. A server with `body-markup`
/// reads the body as markup, so its `&`, `<`, and `>` are escaped there.
#[cfg(any(test, target_os = "linux"))]
pub fn freedesktop(note: &Note, body_markup: bool) -> Notify {
    let body = if body_markup {
        note.body
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    } else {
        note.body.clone()
    };
    Notify {
        app_name: "Hex",
        replaces_id: 0,
        app_icon: APP_ID,
        summary: note.title.clone(),
        body,
        actions: vec![DEFAULT_ACTION.to_owned(), "Open".to_owned()],
        desktop_entry: APP_ID,
        expire_timeout: -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> Note {
        Note {
            title: "Done: Tom & Jerry <draft>".into(),
            body: "Saved \"notes\".txt\u{7}".into(),
            conversation: "c-1".into(),
        }
    }

    #[test]
    fn a_toast_escapes_its_text() {
        assert_eq!(
            toast_xml(&note()),
            "<toast launch=\"hex://chats?id=c-1\"><visual><binding template=\"ToastGeneric\">\
             <text>Done: Tom &amp; Jerry &lt;draft&gt;</text><text>Saved &quot;notes&quot;.txt</text>\
             </binding></visual></toast>"
        );
    }

    #[test]
    fn a_freedesktop_notification_opens_the_chat_by_default() {
        let plain = freedesktop(&note(), false);
        assert_eq!(plain.actions, ["default", "Open"]);
        assert_eq!(plain.body, note().body);
        assert_eq!(plain.summary, "Done: Tom & Jerry <draft>");
        assert_eq!(plain.desktop_entry, "anomaly.hex");
        let marked = freedesktop(
            &Note {
                body: "a < b & c > d".into(),
                ..note()
            },
            true,
        );
        assert_eq!(marked.body, "a &lt; b &amp; c &gt; d");
    }
}
