//! What goes to the person's phone: short, with the task it's about.

use crate::assistant::{Event, Origin};
use crate::desktop::i18n::{tr, tr_fill};

/// The longest text of one message, before "…".
const MAX_CHARS: usize = 600;
/// The longest task title shown.
const TITLE_CHARS: usize = 60;

/// The iMessage for `event` of the task asked as `title`, if it gets one:
/// a question (approvals included), a failure, or a Background job's
/// result. Quick answers to a hold are for a person at the Mac.
pub fn message_for(event: &Event, title: &str) -> Option<String> {
    let title = shorten(title.trim(), TITLE_CHARS);
    match event {
        Event::Question { text, .. } => Some(format!(
            "\u{2753} {}\n{}\n\n{}",
            tr_fill("{} asks:", &title),
            shorten(text.trim(), MAX_CHARS),
            tr(
                "Reply here to answer. „status“ tells how the tasks are doing, „stopp alle“ stops everything."
            )
        )),
        Event::Failed { message, .. } => Some(format!(
            "\u{26a0}\u{fe0f} {}\n{}",
            tr_fill("{} didn't finish:", &title),
            shorten(message.trim(), MAX_CHARS)
        )),
        Event::Answer { text, origin, .. } if *origin != Origin::ActionShortcut => Some(format!(
            "\u{2705} {}\n{}",
            tr_fill("{} is done:", &title),
            shorten(text.trim(), MAX_CHARS)
        )),
        _ => None,
    }
}

/// What the test button shows.
pub fn test_message() -> String {
    tr("Hex: this is how questions and results reach you while you're away. Reply to test it.")
        .to_owned()
}

/// `text` cut to `max` characters, with "…".
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_failures_and_job_results_go_out() {
        let question = Event::Question {
            conversation: "c".into(),
            text: "Which contact is XY?".into(),
        };
        let out = message_for(&question, "Rebuild xyz.com").unwrap();
        assert!(out.contains("Rebuild xyz.com asks:"), "{out}");
        assert!(out.contains("Which contact is XY?"));
        assert!(out.contains("status"));
        let failed = Event::Failed {
            conversation: "c".into(),
            message: "GitHub refused the push".into(),
        };
        assert!(
            message_for(&failed, "Push")
                .unwrap()
                .contains("didn't finish")
        );
        let done = Event::Answer {
            conversation: "c".into(),
            origin: Origin::Background,
            text: "Pushed to github.com/p/site".into(),
        };
        assert!(message_for(&done, "Site").unwrap().contains("is done"));
    }

    #[test]
    fn quick_answers_and_other_events_stay_on_the_mac() {
        let quick = Event::Answer {
            conversation: "c".into(),
            origin: Origin::ActionShortcut,
            text: "It's 18:00".into(),
        };
        assert_eq!(message_for(&quick, "time"), None);
        assert_eq!(
            message_for(
                &Event::Cancelled {
                    conversation: "c".into()
                },
                "x"
            ),
            None
        );
    }

    #[test]
    fn long_text_is_cut() {
        let long = "a".repeat(2_000);
        let failed = Event::Failed {
            conversation: "c".into(),
            message: long,
        };
        let out = message_for(&failed, &"t".repeat(200)).unwrap();
        assert!(out.chars().count() < 800);
        assert!(out.contains('…'));
    }
}
