//! The assistant's shared vocabulary: conversations as stored and shown, a
//! request to answer, and the events a running request reports.
//!
//! This file is the contract between the Action shortcut, the agent, the
//! desktop tools, and the Chats pane. Change it only with all four in mind.

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
    Text { text: String },
    /// A PNG saved beside the conversation (`store::attachment_path`).
    Screenshot { file: String },
    /// One thing the assistant did on the desktop, as a person reads it:
    /// "Clicked Reply", "Typed 12 characters", "Opened Safari".
    Action { description: String },
    /// The request failed here; the text is for a person.
    Error { message: String },
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

/// Where a request came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Held the Action shortcut and spoke: the answer goes to the HUD and is
    /// read aloud.
    ActionShortcut,
    /// Typed in Chats: the answer shows there and is not read aloud.
    Chat,
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
}

/// What the desktop looked like when the question was asked.
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

/// What a running request reports, in order: `Started`, any number of
/// `Thinking`/`Step`/`AnswerDelta`, then exactly one of `Answer`, `Failed`,
/// or `Cancelled`. Every event names its conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Started {
        conversation: ConversationId,
        origin: Origin,
        question: String,
    },
    /// Waiting on the model.
    Thinking { conversation: ConversationId },
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
    Cancelled { conversation: ConversationId },
    /// The stored list changed (a new, renamed, or deleted conversation).
    ConversationsChanged,
}

impl Event {
    /// Its conversation, if it has one.
    pub fn conversation(&self) -> Option<&ConversationId> {
        match self {
            Self::Started { conversation, .. }
            | Self::Thinking { conversation }
            | Self::Step { conversation, .. }
            | Self::AnswerDelta { conversation, .. }
            | Self::Answer { conversation, .. }
            | Self::Failed { conversation, .. }
            | Self::Cancelled { conversation } => Some(conversation),
            Self::ConversationsChanged => None,
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
