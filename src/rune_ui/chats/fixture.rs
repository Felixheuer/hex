//! An in-memory stand-in for the assistant service, which the pane uses
//! until the window connects (scenes, tests), and the scenes' fake
//! conversations, tasks, and setup results.

use std::collections::HashMap;

use crate::assistant::{
    Conversation, ConversationId, ConversationSummary, Message, Origin, Outcome, Part, Request,
    Role, TaskState, TaskStatus,
};

/// Conversations, tasks, and what the pane asked of them.
#[derive(Default)]
pub(in crate::rune_ui) struct Store {
    pub(in crate::rune_ui) conversations: Vec<Conversation>,
    /// How each conversation's last request ended; Answered when unnamed.
    pub(in crate::rune_ui) outcomes: HashMap<ConversationId, Outcome>,
    pub(in crate::rune_ui) tasks: Vec<TaskStatus>,
    /// Every request submitted, in order.
    pub(in crate::rune_ui) submitted: Vec<Request>,
    pub(in crate::rune_ui) cancelled: Vec<ConversationId>,
    pub(in crate::rune_ui) deleted: Vec<ConversationId>,
    /// Take over (`true`) and Hand back (`false`), in order.
    pub(in crate::rune_ui) taken_over: Vec<(ConversationId, bool)>,
    /// Conversations resumed after Hex quit while they ran.
    pub(in crate::rune_ui) resumed: Vec<ConversationId>,
    /// The files in each job folder, by folder.
    pub(in crate::rune_ui) files: HashMap<String, Vec<String>>,
    /// Folders, files, and links opened, in order.
    pub(in crate::rune_ui) opened: Vec<String>,
    /// Scheduled requests.
    pub(in crate::rune_ui) schedules: Vec<super::bridge::schedules::Schedule>,
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
                outcome: self
                    .outcomes
                    .get(&conversation.id)
                    .copied()
                    .unwrap_or_default(),
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
    let downloads = conversation(
        "sort-downloads",
        "Sort my Downloads folder",
        Origin::Background,
        now - 50 * MINUTE,
        vec![
            message(
                Role::User,
                now - 58 * MINUTE,
                vec![text(
                    "Sort my Downloads folder into Invoices, Screenshots, and Installers",
                )],
            ),
            message(
                Role::Assistant,
                now - 50 * MINUTE,
                vec![
                    action("Opened Finder"),
                    action("Made the folder “Invoices”"),
                    action("Moved 12 invoices into “Invoices”"),
                ],
            ),
        ],
    );
    let outcomes = HashMap::from([
        ("book-table".to_owned(), Outcome::Failed),
        ("sort-downloads".to_owned(), Outcome::Interrupted),
    ]);
    use super::bridge::schedules::{Repeat, Schedule};
    let schedules = vec![
        Schedule {
            id: "inbox".into(),
            text: "Summarize my inbox and flag what needs me".into(),
            minute_of_day: 8 * 60,
            repeat: Repeat::Weekdays,
            enabled: true,
            next_ms: None,
        },
        Schedule {
            id: "invoices".into(),
            text: "File last week's invoices in Accounting".into(),
            minute_of_day: 17 * 60 + 30,
            repeat: Repeat::Weekly,
            enabled: false,
            next_ms: None,
        },
    ];
    Store {
        schedules,
        conversations: vec![reply, rename, downloads, regex, weather, failed],
        outcomes,
        tasks: vec![TaskStatus {
            conversation: "reply-to-maya".into(),
            origin: Origin::Background,
            title: "Reply to Maya about Thursday".into(),
            state: TaskState::Running,
            step: Some("Typing the reply in Mail".into()),
            windows: Vec::new(),
            colour: 2,
        }],
        ..Store::default()
    }
}

/// Tasks in every state beside the running one: one taken over, one
/// waiting on a usage limit, one queued.
fn mixed_tasks(store: &mut Store, now: u64) {
    let task = |id: &str, title: &str, state, step: Option<&str>, colour| TaskStatus {
        conversation: id.into(),
        origin: Origin::Background,
        title: title.into(),
        state,
        step: step.map(str::to_owned),
        windows: Vec::new(),
        colour,
    };
    store.tasks.extend([
        task(
            "expenses",
            "File the October expenses",
            TaskState::Paused,
            Some("Filling in the report in Numbers"),
            4,
        ),
        task(
            "pr-summary",
            "Summarize this week's pull requests",
            TaskState::Waiting {
                until_ms: now + 3 * MINUTE + 12_000,
            },
            None,
            3,
        ),
        task(
            "coffee",
            "Order more coffee beans",
            TaskState::Queued,
            None,
            0,
        ),
    ]);
    for (id, title) in [
        ("expenses", "File the October expenses"),
        ("pr-summary", "Summarize this week's pull requests"),
        ("coffee", "Order more coffee beans"),
    ] {
        store.conversations.push(conversation(
            id,
            title,
            Origin::Background,
            now - MINUTE,
            vec![message(Role::User, now - MINUTE, vec![text(title)])],
        ));
        store.outcomes.insert(id.into(), Outcome::Running);
    }
}

/// A big job asking its one question with its plan under way, a finished
/// one with its folder and sheet, and a Here question that became a job.
fn jobs(store: &mut Store, now: u64) {
    let ask = "Research 50 specialty coffee suppliers in Europe, draft an email to each in Gmail, put them in a spreadsheet, document everything, and share it with me in Google Sheets";
    store.conversations.push(conversation(
        "suppliers",
        "Research 50 coffee suppliers and draft emails",
        Origin::Background,
        now - MINUTE,
        vec![message(Role::User, now - 4 * MINUTE, vec![text(ask)])],
    ));
    store.outcomes.insert("suppliers".into(), Outcome::Running);
    store.tasks.push(TaskStatus {
        conversation: "suppliers".into(),
        origin: Origin::Background,
        title: "Research 50 coffee suppliers and draft emails".into(),
        state: TaskState::NeedsInput,
        step: Some("Found 31 of 50 suppliers".into()),
        windows: Vec::new(),
        colour: 5,
    });
    store.conversations.push(conversation(
        "q3-report",
        "Compare the Q3 supplier quotes",
        Origin::Background,
        now - 50 * MINUTE,
        vec![
            message(
                Role::User,
                now - 70 * MINUTE,
                vec![text("Compare the Q3 supplier quotes in my Downloads and share a summary sheet")],
            ),
            message(
                Role::Assistant,
                now - 50 * MINUTE,
                vec![
                    action("Opened 12 quotes in Downloads"),
                    action("Built the comparison in a spreadsheet"),
                    action("Shared it in Google Sheets"),
                    text("Done. **Bean & Co** is cheapest for single origin, **Nordroast** for blends. Everything is in `~/Hex/Jobs/q3-quotes/`, and the shared sheet is at https://docs.google.com/spreadsheets/d/1hX9q3quotes/edit."),
                ],
            ),
        ],
    ));
    store.files.insert(
        "~/Hex/Jobs/q3-quotes/".into(),
        vec![
            "Comparison.xlsx".into(),
            "Quotes.csv".into(),
            "Notes.md".into(),
        ],
    );
}

/// The big job's plan, partway.
fn job_plan() -> Vec<crate::assistant::PlanStep> {
    [
        ("Find 50 specialty coffee suppliers in Europe", false),
        ("Note each one's contact, prices, and minimums", false),
        ("Draft an email to each in Gmail", false),
        ("Build the spreadsheet", false),
        ("Write up how it was done", false),
        ("Share it in Google Sheets", false),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, _))| crate::assistant::PlanStep {
        title: title.into(),
        done: index < 2,
    })
    .collect()
}

/// The onboarding's agent page in `state`.
fn agent_page(ui: &mut super::Ui, state: AgentPage) {
    use crate::app_settings::AssistantBackend;
    super::setup::fixture(ui, super::setup::Page::Agent, false);
    let found = ui.chats.settings.found.get_or_insert_with(Vec::new);
    for entry in found.iter_mut() {
        match state {
            AgentPage::Ready => {
                entry.ready = true;
                entry.note = None;
                if entry.backend != AssistantBackend::ClaudeApi {
                    entry.path = Some(format!("/opt/homebrew/bin/{:?}", entry.backend).into());
                    entry.version.get_or_insert_with(|| "1.0.0".into());
                }
            }
            AgentPage::Nothing => {
                entry.ready = false;
                entry.path = None;
                entry.version = None;
            }
            AgentPage::OwnKey if entry.backend == AssistantBackend::OpenCode => {
                entry.ready = true;
                entry.note = None;
                entry.path = Some("/opt/homebrew/bin/opencode".into());
                entry.version = Some("2.2.6".into());
            }
            AgentPage::OwnKey | AgentPage::Installing => {}
        }
    }
    if state == AgentPage::OwnKey {
        ui.settings.assistant.backend = AssistantBackend::OpenCode;
        ui.settings.assistant.opencode_model = Some("deepseek/deepseek-chat".into());
    }
    let agents = &mut ui.chats.settings.agents;
    match state {
        AgentPage::OwnKey => {
            agents.byok = Some(super::agents::Byok {
                provider: 0,
                model: 0,
                key: rune::TextEdit::new("sk-0f3c9e2b7a5d41c8"),
                verifying: false,
                result: Some(Ok("deepseek/deepseek-chat".into())),
            });
        }
        AgentPage::Installing => {
            ui.settings.assistant.backend = AssistantBackend::Codex;
            agents.working = Some(AssistantBackend::Codex);
            agents.log = vec![
                "==> Downloading codex 0.91.0".into(),
                "==> Pouring codex--0.91.0.arm64_sequoia.bottle.tar.gz".into(),
            ];
        }
        AgentPage::Ready | AgentPage::Nothing => {}
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
        steered: Vec::new(),
        answers: Vec::new(),
        started_ms: crate::history::now_ms() - 3 * MINUTE,
    }
}

/// The agent page's states, for scenes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(in crate::rune_ui) enum AgentPage {
    /// Every agent ready.
    Ready,
    /// Nothing installed: the suggestion.
    Nothing,
    /// OpenCode's own-key panel, DeepSeek's key verified.
    OwnKey,
    /// Codex installing.
    Installing,
}

/// Which Chats scene to set up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(in crate::rune_ui) enum Scene {
    /// The list, a finished conversation with steps, and a running task.
    Conversation,
    /// The running task's transcript, its answer streaming, with a message
    /// sent while it worked.
    Running,
    /// Tasks running, taken over, waiting on a usage limit, and queued.
    Tasks,
    /// A conversation Hex quit in the middle of, with Resume.
    Interrupted,
    /// A big job: its plan, and the one question it asks.
    Job,
    /// A finished job: its folder, files, and shared sheet.
    Results,
    /// A Here question that became a Background job, its plan just in.
    Escalated,
    /// No conversations yet.
    Empty,
    Settings,
    /// The onboarding at `Page`, part way: a check failed, one runs.
    Setup(super::setup::Page),
    /// The onboarding's last page with every check passed.
    SetupDone,
    /// The onboarding as "New in Hex", for someone who set Hex up before.
    NewInHex,
    /// Continuing the voice first run.
    Merged,
    /// Reopened after a restart on the permissions page, checking again.
    Resumed,
    /// Accessibility applies only after a restart: one button does it.
    Restart,
    /// The agent page in each of its states.
    Agent(AgentPage),
    /// The New schedule sheet over Chats.
    NewSchedule,
}

/// Sets `ui` up for a Chats scene: the assistant on, with fake conversations.
pub(in crate::rune_ui) fn scene(ui: &mut super::Ui, scene: Scene) {
    let now = crate::history::now_ms();
    ui.settings.assistant.enabled = true;
    ui.settings.assistant.setup_done = matches!(
        scene,
        Scene::Conversation
            | Scene::Running
            | Scene::Empty
            | Scene::Settings
            | Scene::Tasks
            | Scene::Interrupted
            | Scene::Job
            | Scene::Results
            | Scene::Escalated
            | Scene::NewSchedule
    );
    ui.pane = if matches!(scene, Scene::Settings) {
        super::Pane::Assistant
    } else {
        super::Pane::Chats
    };
    let store = match scene {
        Scene::Empty => Store::default(),
        Scene::Tasks => {
            let mut store = store(now);
            mixed_tasks(&mut store, now);
            store
        }
        Scene::Job | Scene::Results | Scene::Escalated => {
            let mut store = store(now);
            jobs(&mut store, now);
            store
        }
        _ => store(now),
    };
    let mut pane = super::ChatsPane::fixture(store);
    pane.images.fixture();
    pane.settings.displays = super::displays::fixture();
    // The scene shows what it set up, not what following would pick.
    pane.follow.was_showing = true;
    match scene {
        Scene::Conversation
        | Scene::Setup(_)
        | Scene::SetupDone
        | Scene::NewInHex
        | Scene::Merged
        | Scene::Resumed
        | Scene::Restart
        | Scene::Agent(_) => {
            pane.turns.insert("reply-to-maya".into(), running_turn());
            pane.select("rename-screenshots".into());
        }
        Scene::Running => {
            let mut turn = running_turn();
            turn.phase = super::Phase::Answering;
            turn.steered = vec!["Also ask if she wants the projector".into()];
            turn.answer = "I wrote the reply:\n\n> Hi Maya, Thursday at 3 works for me. See you then!\n\nIt's".into();
            pane.turns.insert("reply-to-maya".into(), turn);
            pane.select("reply-to-maya".into());
        }
        Scene::Tasks => {
            pane.turns.insert("reply-to-maya".into(), running_turn());
            let mut expenses = super::Turn::new(
                "File the October expenses".into(),
                Origin::Background,
                super::Phase::Working,
            );
            expenses.steps = vec![
                "Opened Numbers".into(),
                "Filling in the report in Numbers".into(),
            ];
            pane.turns.insert("expenses".into(), expenses);
            pane.holds.insert(
                "pr-summary".into(),
                super::Hold::Waiting {
                    until_ms: now + 3 * MINUTE + 12_000,
                    reason: "Claude Code's usage limit".into(),
                },
            );
            pane.select("expenses".into());
        }
        Scene::Interrupted => {
            pane.turns.insert("reply-to-maya".into(), running_turn());
            pane.select("sort-downloads".into());
        }
        Scene::Job => {
            let mut turn = super::Turn::new(
                "Research 50 specialty coffee suppliers in Europe, draft an email to each in Gmail, put them in a spreadsheet, document everything, and share it with me in Google Sheets".into(),
                Origin::Background,
                super::Phase::Working,
            );
            turn.started_ms = now - (4 * MINUTE + 12_000);
            turn.steps = vec![
                "Searched for specialty coffee roasters and importers".into(),
                "Found 31 of 50 suppliers".into(),
            ];
            pane.turns.insert("suppliers".into(), turn);
            pane.turns.insert("reply-to-maya".into(), running_turn());
            pane.plans.insert("suppliers".into(), job_plan());
            pane.questions.insert(
                "suppliers".into(),
                "I'll draft the emails in your Gmail and share the sheet from your Google account. **Sign in to Google** in Hex's browser so I can, or tell me to save drafts as files instead.".into(),
            );
            pane.select("suppliers".into());
        }
        Scene::Results => {
            pane.select("q3-report".into());
        }
        Scene::NewSchedule => {
            pane.turns.insert("reply-to-maya".into(), running_turn());
            pane.select("rename-screenshots".into());
            pane.new_schedule = Some(super::schedules::NewSchedule {
                text: rune::TextEdit::new("Summarize the news about our competitors"),
                time: rune::TextEdit::new("7:30"),
                repeat: super::bridge::schedules::Repeat::Weekdays,
                error: None,
            });
        }
        Scene::Escalated => {
            let mut turn = super::Turn::new(
                "What does this error mean?".into(),
                Origin::Background,
                super::Phase::Working,
            );
            turn.started_ms = now - 40_000;
            turn.steps = vec!["Read the build log in Xcode".into()];
            turn.answer = "This needs more than a quick answer: I'll fix the signing settings in the background and tell you when it builds.".into();
            pane.turns.insert("window-question".into(), turn);
            pane.plans.insert(
                "window-question".into(),
                vec![
                    crate::assistant::PlanStep {
                        title: "Read the error and the signing settings".into(),
                        done: true,
                    },
                    crate::assistant::PlanStep {
                        title: "Pick the personal team".into(),
                        done: false,
                    },
                    crate::assistant::PlanStep {
                        title: "Build again and check it passes".into(),
                        done: false,
                    },
                ],
            );
            pane.escalated.push("window-question".into());
            pane.select("window-question".into());
        }
        Scene::Empty => {
            pane.tasks.clear();
        }
        Scene::Settings => {
            pane.settings.found = Some(super::setup::fixture_found());
            pane.settings.voices = Some(vec![
                "Ava (Premium)".into(),
                "Samantha".into(),
                "Zoe".into(),
            ]);
            ui.settings.assistant.voice = Some("Ava (Premium)".into());
            pane.settings.memory = rune::TextEdit::new(
                "- Works at Acme as a product designer\n- Manager: Sam\n- Prefers short emails",
            );
            pane.settings.memory_saved = true;
            pane.settings.browser = Some(crate::assistant::browser::AgentBrowser {
                path: Some("/opt/homebrew/bin/agent-browser".into()),
                version: Some("0.9.3".into()),
                browser_ready: true,
            });
            pane.settings.signed_in = true;
            pane.settings.running = Some(
                ["Finder", "Mail", "Messages", "Safari", "Slack"]
                    .map(String::from)
                    .to_vec(),
            );
        }
    }
    ui.chats = pane;
    match scene {
        Scene::Setup(page) => super::setup::fixture(ui, page, false),
        Scene::SetupDone => super::setup::fixture(ui, super::setup::Page::Try, true),
        Scene::NewInHex | Scene::Merged => {
            super::setup::fixture(ui, super::setup::Page::Welcome, false);
            if let Some(flow) = ui.chats.setup.as_mut() {
                flow.new_in_hex = scene == Scene::NewInHex;
                flow.merged = scene == Scene::Merged;
            }
        }
        Scene::Resumed => {
            super::setup::fixture(ui, super::setup::Page::Desktop, false);
            if let Some(flow) = ui.chats.setup.as_mut() {
                flow.resumed = true;
                flow.asked.clear();
                flow.permissions = Some(crate::assistant::computer::Permissions {
                    accessibility: true,
                    screen_recording: true,
                });
                flow.desktop_checking = true;
                flow.desktop = None;
            }
        }
        Scene::Agent(state) => agent_page(ui, state),
        Scene::Restart => {
            super::setup::fixture(ui, super::setup::Page::Desktop, false);
            ui.platform.permissions.accessibility =
                crate::desktop::onboarding::PermissionState::NeedsRestart;
            if let Some(flow) = ui.chats.setup.as_mut() {
                flow.asked = vec![super::setup::Grant::Accessibility];
            }
        }
        _ => {}
    }
}
