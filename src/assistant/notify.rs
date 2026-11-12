//! macOS notifications for Background jobs (`assistant.notifications`): a
//! job finished, failed, or asks something. Inside the bundled, signed
//! Hex.app they go through UNUserNotificationCenter (the
//! `assistant_notifications` Swift bridge), and clicking one opens Chats on
//! its conversation. Outside a bundle (a `hex` binary run from a terminal)
//! they fall back to `osascript`'s `display notification`, which can't open
//! anything when clicked.
//!
//! OWNER: brain.

/// What a notification says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// The conversation a click opens.
    pub conversation: String,
}

/// The notification for a Background job's event, if it gets one.
pub fn for_event(event: &crate::assistant::Event, title: &str) -> Option<Note> {
    use crate::assistant::Event;
    let (heading, body, conversation) = match event {
        Event::Answer {
            conversation, text, ..
        } => ("Done", text.as_str(), conversation),
        Event::Failed {
            conversation,
            message,
        } => ("Didn't finish", message.as_str(), conversation),
        Event::Question { conversation, text } => {
            ("Needs your answer", text.as_str(), conversation)
        }
        _ => return None,
    };
    Some(Note {
        title: format!("{heading}: {}", crate::assistant::store::shorten(title, 50)),
        body: crate::assistant::store::shorten(body, 200),
        conversation: conversation.clone(),
    })
}

/// Shows `note`; returns at once.
pub fn show(note: &Note) {
    #[cfg(target_os = "macos")]
    {
        if bridge::post(note) {
            return;
        }
        let script = format!(
            "display notification {} with title {}",
            applescript(&note.body),
            applescript(&note.title)
        );
        let spawned = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => tracing::warn!(%error, "couldn't show a notification"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = note;
}

/// Registers what a click on a notification does (opening Chats on that
/// conversation): called with the conversation's id. macOS opens a
/// `hex://chats` link instead.
// The platform lane's Windows and Linux notifications call it on a click;
// until they land, the runtime's wiring has nothing to call yet.
#[cfg(not(target_os = "macos"))]
pub fn set_on_click(open: impl Fn(&str) + Send + Sync + 'static) {
    let _ = open;
}

/// `text` as an AppleScript string literal.
fn applescript(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
mod bridge {
    use std::ffi::CString;

    unsafe extern "C" {
        fn hex_assistant_notifications_available() -> bool;
        fn hex_assistant_notify(
            title: *const std::ffi::c_char,
            body: *const std::ffi::c_char,
            conversation: *const std::ffi::c_char,
        ) -> bool;
    }

    /// Posts through UNUserNotificationCenter; false outside a bundle.
    pub(super) fn post(note: &super::Note) -> bool {
        // SAFETY: takes no arguments; reads the main bundle.
        if !unsafe { hex_assistant_notifications_available() } {
            return false;
        }
        let clean = |text: &str| CString::new(text.replace('\0', "")).unwrap_or_default();
        let (title, body, conversation) = (
            clean(&note.title),
            clean(&note.body),
            clean(&note.conversation),
        );
        // SAFETY: three valid NUL-terminated strings that outlive the call;
        // the bridge copies them before returning.
        unsafe { hex_assistant_notify(title.as_ptr(), body.as_ptr(), conversation.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::Event;

    #[test]
    fn a_job_notifies_its_answer_failure_or_question() {
        let note = for_event(
            &Event::Question {
                conversation: "c1".into(),
                text: "Who is your colleague?".into(),
            },
            "Supplier research",
        )
        .unwrap();
        assert_eq!(
            note,
            Note {
                title: "Needs your answer: Supplier research".into(),
                body: "Who is your colleague?".into(),
                conversation: "c1".into(),
            }
        );
        assert!(
            for_event(
                &Event::Thinking {
                    conversation: "c1".into()
                },
                "x"
            )
            .is_none()
        );
        assert_eq!(applescript(r#"say "hi" \ ok"#), r#""say \"hi\" \\ ok""#);
    }
}
