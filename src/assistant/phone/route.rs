//! What a reply from the phone does: stop, tell the status, answer the
//! waiting question, or ask something new — as if typed at the Mac.

use crate::assistant::shortcut::spoken_stop::{SpokenStop, spoken_stop};
use crate::assistant::{Context, ConversationId, Origin, Request, TaskState, TaskStatus};
use crate::desktop::i18n::{tr, tr_fill};

/// What a reply asks for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Route {
    /// „stopp alle“: every task.
    StopAll,
    /// „stopp“: the task whose question waits, if one does.
    Stop(ConversationId),
    /// „status“, „wie weit …“: how the tasks are doing, sent back.
    Status,
    /// The answer to the question that waits.
    Answer(ConversationId),
    /// Anything else: a request of its own (the assistant decides).
    New,
}

/// Where `text` goes, with `waiting` the conversation whose question waits.
pub fn route(text: &str, waiting: Option<&ConversationId>) -> Route {
    match spoken_stop(text) {
        Some(SpokenStop::All) => return Route::StopAll,
        Some(SpokenStop::Quiet) => {
            return match waiting {
                Some(conversation) => Route::Stop(conversation.clone()),
                None => Route::StopAll,
            };
        }
        None => {}
    }
    if asks_status(text) {
        return Route::Status;
    }
    match waiting {
        Some(conversation) => Route::Answer(conversation.clone()),
        None => Route::New,
    }
}

fn asks_status(text: &str) -> bool {
    let text = text.trim().to_lowercase();
    let words = text.split_whitespace().count();
    words <= 8
        && [
            "status",
            "stand",
            "wie weit",
            "was machst du",
            "was macht ihr",
            "bist du fertig",
            "how far",
            "progress",
            "are you done",
        ]
        .iter()
        .any(|phrase| text.contains(phrase))
}

/// The status line sent back: each task, its state and step.
pub fn status_text(tasks: &[TaskStatus]) -> String {
    if tasks.is_empty() {
        return tr("No task is running.").to_owned();
    }
    let lines: Vec<String> = tasks
        .iter()
        .map(|task| {
            let state = match task.state {
                TaskState::Queued => tr("waiting its turn"),
                TaskState::NeedsInput => tr("waiting for your answer"),
                TaskState::Paused => tr("paused"),
                _ => tr("working"),
            };
            let step = task
                .step
                .as_deref()
                .map(|step| format!(" · {}", super::outgoing::shorten(step, 80)))
                .unwrap_or_default();
            format!(
                "• {} — {state}{step}",
                super::outgoing::shorten(&task.title, 60)
            )
        })
        .collect();
    format!(
        "{}\n{}",
        tr_fill("{} tasks:", &tasks.len().to_string()),
        lines.join("\n")
    )
}

/// Carries out a reply from `handle`.
pub fn carry_out(handle: &str, text: &str) {
    let waiting = crate::assistant::inbox()
        .active
        .map(|question| question.conversation);
    match route(text, waiting.as_ref()) {
        Route::StopAll => crate::assistant::stop_all(),
        Route::Stop(conversation) => crate::assistant::cancel(&conversation),
        Route::Status => {
            let status = status_text(&crate::assistant::tasks());
            if let Err(error) = super::send::send(handle, &status) {
                tracing::warn!(%error, "couldn't send the status to the phone");
            }
        }
        Route::Answer(conversation) => {
            crate::assistant::submit(request(text, Some(conversation)));
        }
        Route::New => {
            crate::assistant::submit(request(text, None));
        }
    }
}

fn request(text: &str, conversation: Option<ConversationId>) -> Request {
    Request {
        conversation,
        text: text.trim().to_owned(),
        // As typed: the assistant decides, and a job asks back here.
        origin: Origin::Chat,
        context: Context::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_stop_report_answer_or_ask() {
        let waiting = "c1".to_owned();
        assert_eq!(route("Stopp alle Aufgaben", Some(&waiting)), Route::StopAll);
        assert_eq!(route("Stopp", Some(&waiting)), Route::Stop("c1".into()));
        assert_eq!(route("Stopp", None), Route::StopAll);
        assert_eq!(route("Status?", Some(&waiting)), Route::Status);
        assert_eq!(route("Wie weit bist du?", None), Route::Status);
        assert_eq!(
            route("Ja, privat", Some(&waiting)),
            Route::Answer("c1".into())
        );
        assert_eq!(
            route("Schreib Maya, dass ich später komme", None),
            Route::New
        );
    }

    #[test]
    fn a_long_sentence_mentioning_status_is_a_request() {
        assert_eq!(
            route(
                "Schreib in Slack, dass der Status der Seite jetzt grün ist und alles läuft",
                None
            ),
            Route::New
        );
    }

    #[test]
    fn the_status_lists_each_task() {
        let task = TaskStatus {
            conversation: "c1".into(),
            origin: Origin::Background,
            title: "Rebuild xyz.com".into(),
            state: TaskState::Running,
            step: Some("Comparing screenshots".into()),
            windows: Vec::new(),
            colour: 0,
        };
        let text = status_text(&[task]);
        assert!(
            text.contains("Rebuild xyz.com — working · Comparing screenshots"),
            "{text}"
        );
        assert_eq!(status_text(&[]), "No task is running.");
    }
}
