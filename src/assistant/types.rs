//! The assistant's shared vocabulary: conversations as stored and shown, a
//! request to answer, and the events a running request reports.
//!
//! This file is the contract between the Action shortcut, the agent, the
//! desktop tools, and what shows it (the overlays and the Assistant pane).
//! Change it only with all four in mind.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A conversation's stable id (a UUID-like string, also its file name).
pub type ConversationId = String;

/// Who wrote a message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a message, in the order it happened.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// A PNG saved beside the conversation (`store::attachment_path`).
    Screenshot {
        file: String,
    },
    /// One thing the assistant did on the desktop, as a person reads it:
    /// "Clicked Reply", "Typed 12 characters", "Opened Safari".
    Action {
        description: String,
    },
    /// The request failed here; the text is for a person.
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
    /// Unix milliseconds.
    pub at_ms: u64,
}

impl Message {
    /// Its text parts joined by blank lines.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Where a request came from, which is also its lane.
///
/// - **Here** (`ActionShortcut`): held the Action shortcut. About the window
///   in front; may act in that window only (with real input when allowed,
///   under a "Hands off" pill). One at a time. Answer in the HUD bubble,
///   read aloud.
/// - **Background**: tapped then held the Action shortcut, or said "in the
///   background …". Runs beside the others in windows it opens or claims,
///   never in the window in front, never with real input unless
///   `physical_input`. Listed in the task stack; the screen frame shows
///   while any runs.
/// - **Chat**: typed in Chats; answers there, may act like Background.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    ActionShortcut,
    /// Stored as `agent_mode` before the lanes; still read.
    #[serde(alias = "agent_mode")]
    Background,
    Chat,
}

impl Origin {
    pub fn lane(self) -> Lane {
        match self {
            Self::ActionShortcut => Lane::Here,
            Self::Background | Self::Chat => Lane::Background,
        }
    }
}

/// Where a task may act (`guard` enforces it).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// Only the window that was in front when it was asked.
    Here,
    /// Only windows it opened or claimed; never the window in front.
    Background,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conversation {
    pub id: ConversationId,
    /// A short title from the first question.
    pub title: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub origin: Origin,
    pub messages: Vec<Message>,
}

/// A conversation as the Chats list shows it, without its messages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub title: String,
    pub updated_ms: u64,
    pub origin: Origin,
    /// The last message's text, shortened, for the list's second line.
    pub preview: String,
    /// How its last request ended (`Interrupted` offers Resume).
    pub outcome: Outcome,
}

/// What the desktop looked like when the question was asked (Here lane;
/// Background tasks look for themselves).
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The screen the pointer is on, as PNG (see `computer::Screenshot`).
    pub screenshot: Option<crate::assistant::computer::Screenshot>,
    /// Text selected in the frontmost app, read through Accessibility.
    pub selected_text: Option<String>,
    /// The frontmost app's name, such as "Mail".
    pub frontmost_app: Option<String>,
    /// The focused window's title.
    pub window_title: Option<String>,
    /// The window in front, as agent-desktop names it: the Here lane's only
    /// window.
    pub window: Option<TaskWindow>,
    /// The display the person works on (the one holding the focused window,
    /// which the screenshot shows) and the layout around it, for the prompt:
    /// "The person is working on M27Q (display 5, …). Built-in Retina
    /// Display (display 1, main, …) is below and right of it. …"
    pub display: Option<String>,
}

/// One question to answer.
#[derive(Clone, Debug)]
pub struct Request {
    /// Continue this conversation; `None` starts a new one.
    pub conversation: Option<ConversationId>,
    pub text: String,
    pub origin: Origin,
    pub context: Context,
}

/// A window a task is working in, for its outline on screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskWindow {
    pub app: String,
    /// agent-desktop's window id, when the task named one.
    pub window_id: Option<String>,
}

/// One task as the HUD's stack and Chats show it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskStatus {
    pub conversation: ConversationId,
    pub origin: Origin,
    /// The question, shortened.
    pub title: String,
    pub state: TaskState,
    /// The latest step, as a person reads it.
    pub step: Option<String>,
    pub windows: Vec<TaskWindow>,
    /// Its own colour index (0..8) for its cursor, outline, and stack row.
    pub colour: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    /// Waiting for a free slot (`max_parallel`).
    Queued,
    Running,
    /// The person took it over: its desktop actions wait until handed back.
    Paused,
    /// The backend hit a usage limit; it retries at this Unix ms time.
    Waiting {
        until_ms: u64,
    },
    /// It asked the person something (`Event::Question`) and waits for the
    /// answer: the next hold of the Action shortcut, or a message in Chats.
    NeedsInput,
}

/// A question a task waits on (`TaskState::NeedsInput`), as the overlay's
/// bubble shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaitingQuestion {
    pub conversation: ConversationId,
    /// The task's title.
    pub title: String,
    pub question: String,
    /// When it was asked (Unix ms).
    pub asked_ms: u64,
}

/// One step of a task's plan, as the HUD and Chats show its progress.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanStep {
    pub title: String,
    pub done: bool,
}

/// What a running request reports, in order: `Started` (after any
/// `Queued`), any number of `Thinking`/`Step`/`Window`/`AnswerDelta`, then
/// exactly one of `Answer`, `Failed`, or `Cancelled`. Every event but
/// `ConversationsChanged` and `QuestionsChanged` names its conversation; several
/// conversations' events interleave while tasks run in parallel.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Accepted, waiting for a free slot.
    Queued {
        conversation: ConversationId,
        origin: Origin,
        question: String,
    },
    Started {
        conversation: ConversationId,
        origin: Origin,
        question: String,
    },
    /// Waiting on the model.
    Thinking {
        conversation: ConversationId,
    },
    /// About to do, or doing, one thing on the desktop.
    Step {
        conversation: ConversationId,
        description: String,
    },
    /// More of the answer's text as it streams (may never come).
    AnswerDelta {
        conversation: ConversationId,
        text: String,
    },
    /// The final answer; the conversation is saved.
    Answer {
        conversation: ConversationId,
        origin: Origin,
        text: String,
    },
    Failed {
        conversation: ConversationId,
        message: String,
    },
    /// Started working in a window (its outline appears).
    Window {
        conversation: ConversationId,
        window: TaskWindow,
    },
    Cancelled {
        conversation: ConversationId,
    },
    /// Taken over by the person (`Paused`) or handed back (`Resumed`).
    Paused {
        conversation: ConversationId,
    },
    Resumed {
        conversation: ConversationId,
    },
    /// Hit a usage limit; retries at `until_ms`.
    Waiting {
        conversation: ConversationId,
        until_ms: u64,
        reason: String,
    },
    /// The task's plan, sent whenever it is made or a step changes.
    Plan {
        conversation: ConversationId,
        steps: Vec<PlanStep>,
    },
    /// The task asks the person something before (or while) it works, and
    /// waits (`TaskState::NeedsInput`); read aloud and shown in the bubble
    /// and Chats. The answer arrives as a `submit` into this conversation.
    Question {
        conversation: ConversationId,
        text: String,
    },
    /// A Here request turned out to be a long job and continues as a
    /// Background task in the same conversation (the frame comes on).
    Escalated {
        conversation: ConversationId,
    },
    /// A task called a desktop tool just now: it acts on the screen
    /// (`looking` when it only read or screenshot a window). The screen
    /// frame shows for a few seconds after (`assistant_overlay`).
    Acting {
        conversation: ConversationId,
        looking: bool,
    },
    /// A Here task is (or stops) moving the real pointer or pressing real
    /// keys in the window in front: the HUD shows "Hands off".
    HandsOff {
        conversation: ConversationId,
        on: bool,
    },
    /// The questions waiting on the person changed: which one to answer now
    /// (its task's title and the question, for the overlay's bubble) and
    /// how many wait behind it. Sent when a task asks, is answered, stops,
    /// or the person puts the active one off; `None` when nothing waits.
    QuestionsChanged {
        active: Option<WaitingQuestion>,
        waiting: usize,
    },
    /// The stored list changed (a new, renamed, or deleted conversation).
    ConversationsChanged,
}

impl Event {
    /// Its conversation, if it has one.
    pub fn conversation(&self) -> Option<&ConversationId> {
        match self {
            Self::Queued { conversation, .. }
            | Self::Window { conversation, .. }
            | Self::Started { conversation, .. }
            | Self::Thinking { conversation }
            | Self::Step { conversation, .. }
            | Self::AnswerDelta { conversation, .. }
            | Self::Answer { conversation, .. }
            | Self::Failed { conversation, .. }
            | Self::Cancelled { conversation }
            | Self::Paused { conversation }
            | Self::Resumed { conversation }
            | Self::Waiting { conversation, .. }
            | Self::HandsOff { conversation, .. }
            | Self::Acting { conversation, .. }
            | Self::Plan { conversation, .. }
            | Self::Question { conversation, .. }
            | Self::Escalated { conversation } => Some(conversation),
            Self::ConversationsChanged | Self::QuestionsChanged { .. } => None,
        }
    }

    /// Whether the request is over.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Self::Answer { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

/// A file path beside a conversation (a screenshot it saved).
pub fn attachment(conversation: &str, file: &str) -> Option<PathBuf> {
    crate::assistant::store::attachment_path(conversation, file)
}

/// How a stored conversation's last request ended, for Chats.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Answered,
    Failed,
    Cancelled,
    /// Agent-Desk quit or crashed while it ran; Chats offers Resume.
    Interrupted,
    /// Still running (only while Agent-Desk runs).
    Running,
}
