//! An in-memory stand-in for the assistant service, which the pane uses
//! until the window connects (scenes, tests), and the scenes' fake
//! conversations, tasks, and setup results.

use crate::assistant::{
    Conversation, ConversationId, ConversationSummary, Message, Origin, Part, Request, Role,
    TaskState, TaskStatus,
};

/// Conversations, tasks, and what the pane asked of them.
#[derive(Default)]
pub(in crate::rune_ui) struct Store {
    pub(in crate::rune_ui) conversations: Vec<Conversation>,
    pub(in crate::rune_ui) tasks: Vec<TaskStatus>,
    pub(in crate::rune_ui) agent_mode: bool,
    /// Every request submitted, in order.
    pub(in crate::rune_ui) submitted: Vec<Request>,
    pub(in crate::rune_ui) cancelled: Vec<ConversationId>,
    pub(in crate::rune_ui) deleted: Vec<ConversationId>,
    /// The conversation last named as the one showing.
    pub(in crate::rune_ui) focus: Option<ConversationId>,
}

impl Store {
    /// Newest first, as the service lists them.
    pub(super) fn conversations(&self) -> Vec<ConversationSummary> {
        let mut rows: Vec<_> = self
            .conversations
            .iter()
            .map(|conversation| ConversationSummary {
                id: conversation.id.clone(),
                title: conversation.title.clone(),
                updated_ms: conversation.updated_ms,
                origin: conversation.origin,
                preview: conversation
                    .messages
                    .last()
                    .map(|message| message.text().lines().next().unwrap_or_default().to_owned())
                    .unwrap_or_default(),
                outcome: crate::assistant::Outcome::Answered,
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.updated_ms));
        rows
    }

    pub(super) fn conversation(&self, id: &str) -> Option<Conversation> {
        self.conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .cloned()
    }

    /// Records `request` and names its conversation: the one it continues,
    /// or `chat-<n>`.
    pub(super) fn submit(&mut self, request: Request) -> ConversationId {
        let id = request
            .conversation
            .clone()
            .unwrap_or_else(|| format!("chat-{}", self.submitted.len() + 1));
        self.submitted.push(request);
        id
    }

    pub(super) fn delete(&mut self, id: &str) {
        self.deleted.push(id.to_owned());
        self.conversations
            .retain(|conversation| conversation.id != id);
    }
}

const MINUTE: u64 = 60_000;

fn text(text: &str) -> Part {
    Part::Text { text: text.into() }
}

fn action(description: &str) -> Part {
    Part::Action {
        description: description.into(),
    }
}

fn message(role: Role, at_ms: u64, parts: Vec<Part>) -> Message {
    Message { role, parts, at_ms }
}

fn conversation(
    id: &str,
    title: &str,
    origin: Origin,
    updated_ms: u64,
    messages: Vec<Message>,
) -> Conversation {
    Conversation {
        id: id.into(),
        title: title.into(),
        created_ms: messages.first().map_or(updated_ms, |message| message.at_ms),
        updated_ms,
        origin,
        messages,
    }
}

/// A few conversations from each origin, one with a task still running.
pub(in crate::rune_ui) fn store(now: u64) -> Store {
    let reply = conversation(
        "reply-to-maya",
        "Reply to Maya about Thursday",
        Origin::Background,
        now - 2 * MINUTE,
        vec![message(
            Role::User,
            now - 2 * MINUTE,
            vec![text(
                "Reply to Maya's last email and say Thursday at 3 works for me",
            )],
        )],
    );
    let rename = conversation(
        "rename-screenshots",
        "Rename the screenshots on my desktop",
        Origin::Background,
        now - 26 * MINUTE,
        vec![
            message(
                Role::User,
                now - 28 * MINUTE,
                vec![text(
                    "Rename the screenshots on my desktop by what they show",
                )],
            ),
            message(
                Role::Assistant,
                now - 26 * MINUTE,
                vec![
                    action("Opened Finder"),
                    action("Selected 4 screenshots on the Desktop"),
                    action("Renamed “Screenshot 14.03” to “Deploy button grey”"),
                    action("Renamed “Screenshot 14.05” to “Settings sheet clipped”"),
                    action("Renamed “Screenshot 14.09” to “Build log passed”"),
                    action("Renamed “Screenshot 14.12” to “Release notes draft”"),
                    text(
                        "Done. I renamed **4 screenshots** by what they show:\n\n- Deploy button grey\n- Settings sheet clipped\n- Build log passed\n- Release notes draft",
                    ),
                ],
            ),
        ],
    );
    let regex = conversation(
        "regex",
        "A regex for ISO dates",
        Origin::Chat,
        now - 3 * 60 * MINUTE,
        vec![
            message(
                Role::User,
                now - 3 * 60 * MINUTE - MINUTE,
                vec![text(
                    "What's a regex that matches ISO dates like 2026-11-12?",
                )],
            ),
            message(
                Role::Assistant,
                now - 3 * 60 * MINUTE,
                vec![text(
                    "This matches a year, month, and day:\n\n```\n^\\d{4}-(0[1-9]|1[0-2])-(0[1-9]|[12]\\d|3[01])$\n```\n\nIt checks that the month is **01–12** and the day **01–31**; it doesn't know that February has 28 days.",
                )],
            ),
        ],
    );
    let weather = conversation(
        "window-question",
        "What's this error in Xcode?",
        Origin::ActionShortcut,
        now - 26 * 60 * MINUTE,
        vec![
            message(
                Role::User,
                now - 26 * 60 * MINUTE,
                vec![
                    text("What does this error mean?"),
                    Part::Screenshot {
                        file: "screen.png".into(),
                    },
                ],
            ),
            message(
                Role::Assistant,
                now - 26 * 60 * MINUTE,
                vec![text(
                    "Xcode can't find the **Signing Certificate** for the team you chose. Open *Signing & Capabilities* and pick your personal team, or turn on *Automatically manage signing*.",
                )],
            ),
        ],
    );
    let failed = conversation(
        "book-table",
        "Book a table for Friday",
        Origin::Background,
        now - 4 * 24 * 60 * MINUTE,
        vec![
            message(
                Role::User,
                now - 4 * 24 * 60 * MINUTE,
                vec![text("Book a table for two at Lupa on Friday at 8")],
            ),
            message(
                Role::Assistant,
                now - 4 * 24 * 60 * MINUTE,
                vec![
                    action("Opened Safari"),
                    action("Went to lupa.example"),
                    Part::Error {
                        message: "The booking page asked for a phone number I don't have.".into(),
                    },
                ],
            ),
        ],
    );
    Store {
        conversations: vec![reply, rename, regex, weather, failed],
        tasks: vec![TaskStatus {
            conversation: "reply-to-maya".into(),
            origin: Origin::Background,
            title: "Reply to Maya about Thursday".into(),
            state: TaskState::Running,
            step: Some("Typing the reply in Mail".into()),
            windows: Vec::new(),
            colour: 2,
        }],
        agent_mode: true,
        ..Store::default()
    }
}

/// The running task's turn so far, as its events left it.
pub(in crate::rune_ui) fn running_turn() -> super::Turn {
    super::Turn {
        question: "Reply to Maya's last email and say Thursday at 3 works for me".into(),
        origin: Origin::Background,
        phase: super::Phase::Working,
        steps: vec![
            "Opened Mail".into(),
            "Found Maya's email “Thursday?”".into(),
            "Clicked Reply".into(),
            "Typing the reply in Mail".into(),
        ],
        answer: String::new(),
    }
}

/// Which Chats scene to set up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Scene {
    /// The list, a finished conversation with steps, and a running task.
    Conversation,
    /// The running task's transcript, its answer streaming.
    Running,
    /// No conversations yet.
    Empty,
    Settings,
    /// The onboarding at `Page`, part way: a check failed, one runs.
    Setup(super::setup::Page),
    /// The onboarding's last page with every check passed.
    SetupDone,
}

/// Sets `ui` up for a Chats scene: the Lab on, with fake conversations.
pub(in crate::rune_ui) fn scene(ui: &mut super::Ui, scene: Scene) {
    let now = crate::history::now_ms();
    ui.settings.labs.assistant = true;
    ui.settings.assistant.setup_done = !matches!(scene, Scene::Setup(_) | Scene::SetupDone);
    ui.pane = super::Pane::Chats;
    let store = if scene == Scene::Empty {
        Store::default()
    } else {
        store(now)
    };
    let mut pane = super::ChatsPane::fixture(store);
    pane.images.fixture();
    match scene {
        Scene::Conversation | Scene::Setup(_) | Scene::SetupDone => {
            pane.turns.insert("reply-to-maya".into(), running_turn());
            pane.select("rename-screenshots".into());
        }
        Scene::Running => {
            let mut turn = running_turn();
            turn.phase = super::Phase::Answering;
            turn.answer = "I wrote the reply:\n\n> Hi Maya, Thursday at 3 works for me. See you then!\n\nIt's".into();
            pane.turns.insert("reply-to-maya".into(), turn);
            pane.select("reply-to-maya".into());
        }
        Scene::Empty => {
            pane.tasks.clear();
            pane.agent_mode = false;
        }
        Scene::Settings => {
            pane.view = super::View::Settings;
            pane.settings.found = Some(super::setup::fixture_found());
            pane.settings.voices = Some(vec![
                "Ava (Premium)".into(),
                "Samantha".into(),
                "Zoe".into(),
            ]);
            ui.settings.assistant.voice = Some("Ava (Premium)".into());
        }
    }
    ui.chats = pane;
    match scene {
        Scene::Setup(page) => super::setup::fixture(ui, page, false),
        Scene::SetupDone => super::setup::fixture(ui, super::setup::Page::Try, true),
        _ => {}
    }
}
