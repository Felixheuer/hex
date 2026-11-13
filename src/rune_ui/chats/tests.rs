//! Chats and the assistant's onboarding, driven through the window as a
//! person would, over the in-memory store (no service, no data folder).

use rune::text::TextSystem;
use rune::{Env, Key, KeyEvent, Mods, Runtime, TextEdit};

use super::super::scenes::SCENES;
use super::super::{Pane, Service, Ui, offscreen, on_key, poll, view};
use super::setup::{Grant, Page, State};
use super::{Done, Job};
use crate::app_settings::{AppSettings, AssistantBackend};
use crate::assistant::backends::{Check, CheckResult, CheckState};
use crate::assistant::computer::Permissions;
use crate::assistant::{Event, Origin};

const SIZE: (f32, f32) = (980.0, 760.0);

fn runtime_for(state: Ui) -> Runtime<Ui> {
    let env = Env {
        system_dark: true,
        scale: 2.0,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = offscreen::runtime(state, view, SIZE, env, text);
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn scene(name: &str) -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    let (_, apply) = SCENES
        .iter()
        .find(|(scene, _)| *scene == name)
        .unwrap_or_else(|| panic!("no scene {name}"));
    apply(&mut state);
    runtime_for(state)
}

fn settle(rt: &mut Runtime<Ui>) {
    for _ in 0..60 {
        rt.frame(1.0 / 30.0);
    }
    rt.settle_all();
    rt.frame(0.0);
}

/// Scrolls the stream until `key` shows, as a person would with the wheel.
fn reveal(rt: &mut Runtime<Ui>, key: &str) {
    for _ in 0..300 {
        let elements = rt.elements();
        let Some(target) = elements
            .iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        else {
            return;
        };
        let inside = format!("/{}/", super::follow::SCROLL);
        let Some(view) = elements
            .iter()
            .find(|e| e.key.as_deref() == Some(super::follow::SCROLL))
            .filter(|_| target.path.contains(&inside))
        else {
            return;
        };
        let middle = target.rect[1] + target.rect[3] / 2.0;
        let (top, bottom) = (view.rect[1] + 8.0, view.rect[1] + view.rect[3] - 8.0);
        if middle > top && middle < bottom {
            return;
        }
        // Positive moves the content down, to what is above.
        let dy = if middle >= bottom { -40.0 } else { 40.0 };
        rt.wheel(
            view.rect[0] + view.rect[2] / 2.0,
            view.rect[1] + view.rect[3] / 2.0,
            dy,
        );
        rt.frame(1.0 / 60.0);
    }
}

fn tap(rt: &mut Runtime<Ui>, key: &str) {
    reveal(rt, key);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn texts(rt: &Runtime<Ui>) -> Vec<String> {
    rt.elements().into_iter().filter_map(|e| e.text).collect()
}

fn shows(rt: &Runtime<Ui>, words: &str) -> bool {
    texts(rt).iter().any(|text| text == words)
}

/// A Chats scene with its setup done and nothing selected.
fn chats() -> Runtime<Ui> {
    scene("mac-chats")
}

fn answer(rt: &mut Runtime<Ui>, done: Done) {
    rt.state.chats.jobs.finish(done);
    rt.invalidate();
    settle(rt);
}

#[test]
fn chats_stays_in_the_sidebar_while_the_assistant_is_off_and_offers_to_turn_it_on() {
    let mut rt = chats();
    assert!(rt.find("nav-chats").is_some());
    rt.state.settings.assistant.enabled = false;
    rt.invalidate();
    settle(&mut rt);
    assert!(rt.find("nav-chats").is_some(), "Chats stays in the sidebar");
    assert_eq!(rt.state.pane, Pane::Chats, "the window stays on Chats");
    assert!(shows(&rt, "The assistant is off"));
    assert!(rt.find("new-chat").is_none(), "no chat while it's off");
    tap(&mut rt, "turn-on-assistant");
    assert!(rt.state.settings.assistant.enabled);
    assert!(!shows(&rt, "The assistant is off"));
    assert!(rt.state.chats.setup.is_none(), "its setup was already done");
}

#[test]
fn the_settings_switch_turns_the_assistant_off_and_on() {
    let mut rt = scene("mac-chats-settings");
    assert!(shows(&rt, "Hold right ⌘ to ask, double-tap for your tasks"));
    assert!(rt.find("run-assistant-setup").is_some());
    tap(&mut rt, "assistant-enabled");
    assert!(!rt.state.settings.assistant.enabled);
    assert!(!crate::assistant::enabled(&rt.state.settings));
    assert!(
        rt.find("run-assistant-setup").is_none(),
        "nothing to set up while off"
    );
    tap(&mut rt, "assistant-enabled");
    assert!(rt.state.settings.assistant.enabled);
}

/// The assistant's shortcuts are recorded where its settings are, with the
/// same recorder General uses, and not in General too.
#[test]
fn assistant_settings_record_the_assistants_shortcuts() {
    let mut rt = scene("mac-chats-settings");
    for key in ["action-shortcut", "stop-all-shortcut"] {
        reveal(&mut rt, key);
        assert!(rt.find(key).is_some(), "{key} in Assistant settings");
    }
    let general = scene("mac-general");
    assert!(general.find("action-shortcut").is_none(), "not in General");
}

#[test]
fn the_list_groups_conversations_and_the_transcript_renders_them() {
    let rt = chats();
    for words in [
        "Today",
        "Yesterday",
        "Reply to Maya about Thursday",
        "A regex for ISO dates",
        "Rename the screenshots on my desktop",
        "6 steps",
    ] {
        assert!(shows(&rt, words), "shows {words:?}: {:?}", texts(&rt));
    }
    // Markdown: the answer's list items, without their marks.
    assert!(shows(&rt, "Release notes draft"));
    // The preview line drops Markdown's asterisks.
    assert!(
        texts(&rt)
            .iter()
            .any(|text| text.starts_with("Done. I renamed 4 screenshots")),
        "{:?}",
        texts(&rt)
    );
    // The running task: in the strip, with Stop.
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref().is_some_and(|key| key.starts_with("task-")))
    );
}

#[test]
fn selecting_a_conversation_names_it_to_the_service_and_leaving_clears_it() {
    let mut rt = chats();
    tap(&mut rt, "chat-regex");
    assert_eq!(rt.state.chats.selected.as_deref(), Some("regex"));
    let store = rt.state.chats.store().unwrap();
    assert_eq!(store.focus.as_deref(), Some("regex"));
    tap(&mut rt, "nav-history");
    assert_eq!(rt.state.chats.store().unwrap().focus, None);
}

#[test]
fn the_composer_sends_with_return_and_continues_the_shown_chat() {
    let mut rt = chats();
    tap(&mut rt, "chat-regex");
    rt.state.chats.composer = TextEdit::new("And for times?");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-input");
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    settle(&mut rt);
    let store = rt.state.chats.store().unwrap();
    assert_eq!(store.submitted.len(), 1);
    let sent = &store.submitted[0];
    assert_eq!(sent.text, "And for times?");
    assert_eq!(sent.origin, Origin::Chat);
    assert_eq!(sent.conversation.as_deref(), Some("regex"));
    assert!(rt.state.chats.composer.text.is_empty());
    assert!(shows(&rt, "Sending…"), "the request shows at once");
}

#[test]
fn shift_return_keeps_typing_and_a_message_is_a_request_of_its_own() {
    let mut rt = chats();
    assert!(rt.find("new-chat").is_none(), "one stream: no New chat");
    assert_eq!(rt.state.chats.target, None);
    rt.state.chats.composer = TextEdit::new("Hello");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-input");
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: shift,
    });
    settle(&mut rt);
    assert!(rt.state.chats.store().unwrap().submitted.is_empty());
    tap(&mut rt, "composer-send");
    let store = rt.state.chats.store().unwrap();
    assert_eq!(store.submitted.len(), 1);
    assert_eq!(store.submitted[0].conversation, None);
    assert_eq!(rt.state.chats.selected.as_deref(), Some("chat-1"));
}

#[test]
fn an_answer_streams_in_and_lands_in_the_stored_conversation() {
    let mut rt = chats();
    rt.state.chats.composer = TextEdit::new("What's 2 + 2?");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-send");
    let id = "chat-1".to_owned();
    for event in [
        Event::Started {
            conversation: id.clone(),
            origin: Origin::Chat,
            question: "What's 2 + 2?".into(),
        },
        Event::Step {
            conversation: id.clone(),
            description: "Opened Calculator".into(),
        },
        Event::AnswerDelta {
            conversation: id.clone(),
            text: "It's **4**".into(),
        },
    ] {
        rt.state.chats.deliver(event);
    }
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Opened Calculator"));
    assert!(shows(&rt, "Writing…"));
    assert!(
        rt.find("composer-stop").is_none(),
        "the composer asks anew until an entry is clicked"
    );
    tap(&mut rt, "chat-chat-1");
    assert_eq!(rt.state.chats.target.as_deref(), Some("chat-1"));
    assert!(
        rt.find("composer-stop").is_some(),
        "Stop replaces Send while it runs"
    );
    tap(&mut rt, "composer-stop");
    assert_eq!(rt.state.chats.store().unwrap().cancelled, vec![id.clone()]);
    rt.state.chats.deliver(Event::Failed {
        conversation: id.clone(),
        message: "The agent stopped".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(!rt.state.chats.turns.contains_key(&id));
    assert!(shows(&rt, "The agent stopped"));
}

#[test]
fn delete_asks_first_and_escape_disarms_it() {
    let mut rt = chats();
    tap(&mut rt, "chat-regex");
    tap(&mut rt, "delete-chat");
    assert!(rt.find("confirm-delete").is_some());
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    settle(&mut rt);
    assert!(rt.find("confirm-delete").is_none());
    tap(&mut rt, "delete-chat");
    tap(&mut rt, "confirm-delete");
    assert_eq!(
        rt.state.chats.store().unwrap().deleted,
        vec!["regex".to_owned()]
    );
    assert_eq!(rt.state.chats.selected, None);
    assert!(!shows(&rt, "A regex for ISO dates"));
}

#[test]
fn the_header_counts_tasks_and_stop_all_and_take_over_reach_the_service() {
    let mut rt = chats();
    assert!(rt.find("agent-mode").is_none(), "Agent mode is gone");
    assert!(shows(&rt, "1 running"), "{:?}", texts(&rt));
    tap(&mut rt, "take-over");
    assert_eq!(
        rt.state.chats.store().unwrap().taken_over,
        vec![("reply-to-maya".to_owned(), true)]
    );
    assert!(shows(&rt, "You have it"));
    tap(&mut rt, "hand-back");
    assert_eq!(
        rt.state.chats.store().unwrap().taken_over.last(),
        Some(&("reply-to-maya".to_owned(), false))
    );
    tap(&mut rt, "stop-task");
    assert_eq!(
        rt.state.chats.store().unwrap().cancelled,
        vec!["reply-to-maya".to_owned()]
    );
    tap(&mut rt, "stop-all-tasks");
    assert_eq!(rt.state.chats.store().unwrap().cancelled.len(), 2);
}

#[test]
fn tasks_show_taken_over_waiting_and_queued_and_events_move_them() {
    let mut rt = scene("mac-chats-tasks");
    assert!(shows(&rt, "4 running"), "{:?}", texts(&rt));
    assert!(shows(&rt, "Queued"));
    assert!(
        texts(&rt)
            .iter()
            .any(|text| text.starts_with("Retries in 3:")),
        "{:?}",
        texts(&rt)
    );
    // The list marks how conversations ended.
    assert!(shows(&rt, "Failed"));
    assert!(shows(&rt, "Interrupted: Agent-Desk quit while it ran"));
    rt.state.chats.deliver(Event::Waiting {
        conversation: "reply-to-maya".into(),
        until_ms: crate::history::now_ms() + 60_000,
        reason: "Codex's usage limit".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(
        texts(&rt)
            .iter()
            .any(|text| text.contains("Codex's usage limit")),
        "{:?}",
        texts(&rt)
    );
    rt.state.chats.deliver(Event::Step {
        conversation: "reply-to-maya".into(),
        description: "Clicked Send".into(),
    });
    rt.state.chats.deliver(Event::Paused {
        conversation: "coffee".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(
        rt.state.chats.view.hold("reply-to-maya").is_none(),
        "retrying"
    );
    assert_eq!(
        rt.state.chats.view.hold("coffee"),
        Some(super::Hold::Paused)
    );
    rt.state.chats.deliver(Event::Resumed {
        conversation: "coffee".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(rt.state.chats.view.hold("coffee").is_none());
}

#[test]
fn an_interrupted_conversation_offers_resume() {
    let mut rt = scene("mac-chats-interrupted");
    assert!(shows(&rt, "This task stopped when Agent-Desk quit"));
    tap(&mut rt, "resume");
    assert_eq!(
        rt.state.chats.store().unwrap().resumed,
        vec!["sort-downloads".to_owned()]
    );
    assert!(rt.state.chats.running("sort-downloads"));
    assert!(
        rt.find("resume").is_none(),
        "the banner leaves while it runs"
    );
}

#[test]
fn the_composer_sends_auto_by_default_with_here_and_background_overrides() {
    use super::SendMode;
    let mut rt = chats();
    assert!(rt.find("composer-send-mode-control").is_none() || shows(&rt, "Auto"));
    for (mode, words) in [
        (SendMode::Auto, "Summarize this page"),
        (SendMode::Here, "What's this error?"),
        (SendMode::Background, "Sort my Downloads"),
    ] {
        rt.state.chats.lane = mode;
        rt.state.chats.composer = TextEdit::new(words);
        rt.invalidate();
        settle(&mut rt);
        tap(&mut rt, "composer-send");
    }
    let store = rt.state.chats.store().unwrap();
    let window = |index: usize| {
        store.submitted[index]
            .context
            .window
            .as_ref()
            .map(|window| window.app.clone())
    };
    // Auto: the assistant decides, knowing the window in front of Agent-Desk.
    assert_eq!(store.submitted[0].origin, Origin::Chat);
    assert_eq!(window(0).as_deref(), Some("Safari"));
    assert_eq!(store.submitted[1].origin, Origin::ActionShortcut);
    assert_eq!(window(1).as_deref(), Some("Safari"));
    assert_eq!(store.submitted[2].origin, Origin::Chat);
    assert_eq!(window(2), None, "Background looks for itself");
}

#[test]
fn a_job_shows_its_plan_and_its_question_takes_the_answer() {
    let mut rt = scene("mac-chats-job");
    assert!(shows(&rt, "Plan"));
    assert!(shows(&rt, "Draft an email to each in Gmail"));
    assert!(shows(&rt, "Sign in to Google…"));
    tap(&mut rt, "plan-head");
    assert!(!shows(&rt, "Build the spreadsheet"), "folds to one line");
    rt.state.chats.answer = TextEdit::new("Save drafts as files");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "send-answer");
    let store = rt.state.chats.store().unwrap();
    let sent = store.submitted.last().unwrap();
    assert_eq!(sent.conversation.as_deref(), Some("suppliers"));
    assert_eq!(sent.text, "Save drafts as files");
    assert!(rt.state.chats.question("suppliers").is_none());
    assert!(
        shows(&rt, "Save drafts as files"),
        "the answer shows under it"
    );
    rt.state.chats.deliver(Event::Question {
        conversation: "suppliers".into(),
        text: "Which countries first?".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Which countries first?"));
}

#[test]
fn a_finished_job_offers_its_folder_files_and_sheet() {
    let mut rt = scene("mac-chats-results");
    for words in ["Open folder", "Comparison.xlsx", "Open Google Sheet"] {
        assert!(shows(&rt, words), "{words}: {:?}", texts(&rt));
    }
    tap(&mut rt, "link0");
    tap(&mut rt, "file0");
    assert_eq!(
        rt.state.chats.store().unwrap().opened,
        vec![
            "https://docs.google.com/spreadsheets/d/1hX9q3quotes/edit".to_owned(),
            "~/Agent-Desk/Jobs/q3-quotes/Comparison.xlsx".to_owned(),
        ]
    );
}

#[test]
fn a_here_question_that_becomes_a_job_shows_one_state() {
    let mut rt = chats();
    rt.state.chats.deliver(Event::Started {
        conversation: "window-question".into(),
        origin: Origin::ActionShortcut,
        question: "Fix this error".into(),
    });
    rt.state.chats.deliver(Event::Escalated {
        conversation: "window-question".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert_eq!(rt.state.chats.selected.as_deref(), Some("window-question"));
    assert_eq!(
        rt.state.chats.turns["window-question"].origin,
        Origin::Background
    );
    // One state per entry: it runs; no second chip says it moved.
    assert!(shows(&rt, "Running"));
    assert!(!shows(&rt, "Moved to the background"));
}

#[test]
fn a_message_to_a_running_conversation_steers_it() {
    let mut rt = chats();
    tap(&mut rt, "chat-reply-to-maya");
    rt.state.chats.composer = TextEdit::new("Also ask about the projector");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-send");
    let store = rt.state.chats.store().unwrap();
    assert_eq!(store.submitted.len(), 1);
    assert_eq!(
        store.submitted[0].conversation.as_deref(),
        Some("reply-to-maya"),
        "into the same conversation"
    );
    assert_eq!(store.submitted[0].origin, Origin::Background);
    let turn = &rt.state.chats.turns["reply-to-maya"];
    assert_eq!(
        turn.steered,
        vec!["Also ask about the projector".to_owned()]
    );
    assert_eq!(
        rt.state.chats.in_flight("reply-to-maya").unwrap().log.len(),
        4,
        "the turn goes on"
    );
    assert!(shows(&rt, "Sent while working"));
}

/// The pane over the fixture store with `tasks`, nothing selected yet.
fn pane_with(tasks: Vec<crate::assistant::TaskStatus>) -> super::ChatsPane {
    let mut store = super::fixture::store(crate::history::now_ms());
    store.tasks = tasks;
    super::ChatsPane::fixture(store)
}

fn task(
    id: &str,
    origin: Origin,
    state: crate::assistant::TaskState,
) -> crate::assistant::TaskStatus {
    crate::assistant::TaskStatus {
        conversation: id.into(),
        origin,
        title: id.into(),
        state,
        step: None,
        windows: Vec::new(),
        colour: 0,
    }
}

#[test]
fn opening_chats_selects_here_then_the_latest_started_task_then_the_latest_chat() {
    use crate::assistant::TaskState::{Queued, Running};
    let mut pane = pane_with(vec![
        task("regex", Origin::Background, Running),
        task("window-question", Origin::ActionShortcut, Running),
        task("book-table", Origin::Background, Running),
    ]);
    pane.auto_select();
    assert_eq!(
        pane.selected.as_deref(),
        Some("window-question"),
        "Here first"
    );
    let mut pane = pane_with(vec![
        task("regex", Origin::Background, Running),
        task("book-table", Origin::Background, Running),
        task("sort-downloads", Origin::Background, Queued),
    ]);
    pane.auto_select();
    assert_eq!(
        pane.selected.as_deref(),
        Some("book-table"),
        "the most recently started, not the queued"
    );
    let mut pane = pane_with(Vec::new());
    pane.auto_select();
    assert_eq!(
        pane.selected.as_deref(),
        Some("reply-to-maya"),
        "else the latest conversation"
    );
}

#[test]
fn chats_follows_new_activity_until_a_pick_then_offers_jump_to_current() {
    let mut rt = chats();
    rt.update(|ui, _| ui.chats.auto_select());
    let start = |rt: &mut Runtime<Ui>, id: &str| {
        rt.state.chats.deliver(Event::Started {
            conversation: id.into(),
            origin: Origin::Background,
            question: format!("Task {id}"),
        });
        rt.invalidate();
        settle(rt);
    };
    start(&mut rt, "new-1");
    assert_eq!(rt.state.chats.selected.as_deref(), Some("new-1"), "follows");
    tap(&mut rt, "chat-regex");
    start(&mut rt, "new-2");
    assert_eq!(
        rt.state.chats.selected.as_deref(),
        Some("regex"),
        "a pick stays put"
    );
    assert!(rt.find("jump-to-current").is_some());
    tap(&mut rt, "jump-to-current");
    assert_eq!(rt.state.chats.selected.as_deref(), Some("new-2"));
    assert!(rt.find("jump-to-current").is_none());
    // Following again; typing holds it back.
    rt.state.chats.composer = TextEdit::new("Half a thought");
    rt.invalidate();
    settle(&mut rt);
    start(&mut rt, "new-3");
    assert_eq!(rt.state.chats.selected.as_deref(), Some("new-2"));
    assert_eq!(rt.state.chats.follow.elsewhere.as_deref(), Some("new-3"));
}

#[test]
fn chats_shows_a_finish_setup_card_until_ready() {
    let mut rt = chats();
    rt.state.settings.assistant.setup_done = false;
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Finish setting up the assistant"));
    assert!(
        rt.find("composer-input").is_none(),
        "no chat until it's ready"
    );
    tap(&mut rt, "continue-setup");
    assert!(rt.state.chats.setup.is_some());
}

#[test]
fn a_hold_before_setup_opens_the_onboarding_and_says_so() {
    let mut rt = chats();
    rt.state.settings.assistant.setup_done = false;
    rt.update(super::open_setup_from_hold);
    settle(&mut rt);
    assert!(rt.state.chats.setup.as_ref().unwrap().from_hold);
    assert!(shows(&rt, "Finish setting up the assistant first"));
}

#[test]
fn turning_the_assistant_on_opens_the_onboarding_until_it_is_done() {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    super::fixture_scene(&mut state, super::fixture::Scene::Empty);
    state.settings.assistant.enabled = false;
    state.settings.assistant.setup_done = false;
    let mut rt = runtime_for(state);
    assert!(rt.state.chats.setup.is_none(), "off, nothing to set up");
    tap(&mut rt, "turn-on-assistant");
    assert!(rt.state.settings.assistant.enabled);
    assert!(rt.state.chats.setup.is_some());
    assert!(shows(&rt, "Meet your assistant"));
    // It starts looking at once; nothing shows ✓ before an answer.
    let asked = &rt.state.chats.jobs.asked;
    assert!(asked.contains(&Job::Detect) && asked.contains(&Job::Desktop));
    assert!(asked.contains(&Job::Permissions));
}

/// The onboarding, opened on a fresh fixture (nothing checked yet).
fn onboarding() -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    super::fixture_scene(&mut state, super::fixture::Scene::Empty);
    state.settings.assistant.setup_done = false;
    super::open_setup(&mut state);
    runtime_for(state)
}

fn page(rt: &Runtime<Ui>) -> Page {
    rt.state.chats.setup.as_ref().unwrap().page
}

#[test]
fn continue_waits_for_each_page_and_later_marks_it_unfinished() {
    let mut rt = onboarding();
    tap(&mut rt, "continue");
    assert_eq!(page(&rt), Page::Privacy);
    tap(&mut rt, "continue");
    assert_eq!(page(&rt), Page::Agent);
    // Nothing found yet: Continue stays put.
    tap(&mut rt, "continue");
    assert_eq!(page(&rt), Page::Agent);
    answer(&mut rt, Done::Detected(super::setup::fixture_found()));
    assert!(shows(&rt, "Ready"));
    tap(&mut rt, "continue");
    assert_eq!(page(&rt), Page::Desktop);
    // Screen Recording unread: Later moves on and remembers.
    tap(&mut rt, "later");
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert_eq!(flow.page, Page::Browser);
    assert_eq!(flow.later, vec![Page::Desktop]);
}

#[test]
fn a_permission_waits_after_allow_until_macos_reports_it() {
    let mut rt = onboarding();
    rt.state.host = super::super::platform::PlatformHost::Fixture(Vec::new());
    rt.state.platform.permissions.microphone = crate::desktop::onboarding::PermissionState::Ready;
    rt.state.platform.permissions.accessibility =
        crate::desktop::onboarding::PermissionState::Ready;
    rt.update(|ui, _| super::setup::go(ui, Page::Desktop));
    answer(
        &mut rt,
        Done::Permissions(Permissions {
            accessibility: true,
            screen_recording: false,
        }),
    );
    let state = |rt: &Runtime<Ui>| {
        let flow = rt.state.chats.setup.as_ref().unwrap();
        super::setup::grant_state(&rt.state, flow, Grant::ScreenRecording)
    };
    assert_eq!(state(&rt), State::Failed);
    tap(&mut rt, "allow");
    assert_eq!(state(&rt), State::Waiting);
    let super::super::platform::PlatformHost::Fixture(sent) = &rt.state.host else {
        panic!("fixture host");
    };
    assert!(
        sent.contains(&super::super::platform::PlatformRequest::Permission(
            crate::desktop::onboarding::PermissionAction::ShowGuide(
                crate::desktop::onboarding::PermissionKind::ScreenRecording
            )
        ))
    );
    answer(
        &mut rt,
        Done::Permissions(Permissions {
            accessibility: true,
            screen_recording: true,
        }),
    );
    assert_eq!(state(&rt), State::Passed);
    // The voice setup's grants show only while missing.
    assert!(rt.find("grant-microphone").is_none());
    assert!(rt.find("grant-screen-recording").is_some());
}

#[test]
fn a_grant_that_needs_a_restart_relaunches_hex_in_one_click() {
    let mut rt = scene("mac-assistant-setup-restart");
    assert!(shows(&rt, "Restart Agent-Desk and continue"));
    tap(&mut rt, "restart");
    let asked = match &rt.state.host {
        crate::rune_ui::platform::PlatformHost::Fixture(sent) => sent.clone(),
        _ => Vec::new(),
    };
    assert!(
        asked.contains(&crate::rune_ui::platform::PlatformRequest::Permission(
            crate::desktop::onboarding::PermissionAction::Restart
        ))
    );
}

#[test]
fn a_failed_check_shows_its_fix_while_later_checks_still_run() {
    let rt = scene("mac-assistant-setup-checks");
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert!(flow.checks_running);
    assert!(
        shows(&rt, "Open Screen Recording…"),
        "the fix shows at once: {:?}",
        texts(&rt)
    );
}

#[test]
fn the_assistant_pages_continue_the_voice_first_run_before_hex_is_ready() {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    let (_, apply) = SCENES
        .iter()
        .find(|(scene, _)| *scene == "mac-onboarding-try")
        .unwrap();
    apply(&mut state);
    state.settings.assistant.enabled = true;
    state.settings.assistant.setup_done = false;
    let mut rt = runtime_for(state);
    settle(&mut rt);
    assert!(rt.state.chats.setup.is_none(), "the voice pages come first");
    // The story reaches its end: it holds there, and the assistant's pages
    // open over it.
    rt.state.first_run.as_mut().unwrap().jump_to_ready();
    rt.invalidate();
    settle(&mut rt);
    let flow = rt
        .state
        .chats
        .setup
        .as_ref()
        .expect("the assistant's pages");
    assert!(flow.merged);
    assert!(shows(&rt, "Now, your assistant"));
    assert!(!rt.state.first_run.as_ref().unwrap().ready_age().is_finite());
    // Done (or left for later): "Agent-Desk is ready" ends both.
    rt.update(|ui, _| super::setup::close(ui));
    settle(&mut rt);
    assert!(rt.state.chats.setup.is_none());
    assert!(rt.state.first_run.as_ref().unwrap().ready_age().is_finite());
    // It doesn't hold again.
    settle(&mut rt);
    assert!(rt.state.chats.setup.is_none());
}

#[test]
fn with_nothing_installed_one_button_sets_up_the_suggested_agent() {
    let mut rt = scene("mac-assistant-setup-agent-none");
    assert!(shows(&rt, "Recommended: Claude Code"));
    tap(&mut rt, "set-up-recommended");
    assert!(
        rt.state
            .chats
            .jobs
            .asked
            .iter()
            .any(|job| matches!(job, Job::SetUpAgent(AssistantBackend::ClaudeCode, _)))
    );
    assert_eq!(
        rt.state.chats.settings.agents.working,
        Some(AssistantBackend::ClaudeCode)
    );
    // Its lines show as it goes, and the row turns Ready when it answers.
    answer(
        &mut rt,
        Done::AgentProgress(
            AssistantBackend::ClaudeCode,
            "Downloading Claude Code".into(),
        ),
    );
    assert!(shows(&rt, "Downloading Claude Code"));
    answer(
        &mut rt,
        Done::AgentSetUp(AssistantBackend::ClaudeCode, Ok(())),
    );
    assert_eq!(rt.state.chats.settings.agents.working, None);
}

#[test]
fn your_own_key_installs_opencode_checks_the_key_and_saves_the_model() {
    let mut rt = scene("mac-assistant-setup-agent-none");
    tap(&mut rt, "use-own-key");
    assert!(shows(&rt, "Your own API key"));
    tap(&mut rt, "provider-deepseek");
    rt.state.chats.settings.agents.byok.as_mut().unwrap().key = TextEdit::new("sk-deepseek-123456");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "verify-key");
    assert!(rt.state.chats.jobs.asked.iter().any(|job| matches!(
        job,
        Job::OwnKey { provider, install: true, .. } if provider == "deepseek"
    )));
    answer(&mut rt, Done::OwnKey(Ok("deepseek/deepseek-chat".into())));
    assert_eq!(
        rt.state.settings.assistant.backend,
        AssistantBackend::OpenCode
    );
    assert_eq!(
        rt.state.settings.assistant.opencode_model.as_deref(),
        Some("deepseek/deepseek-chat")
    );
    assert!(shows(
        &rt,
        "Works. OpenCode answers with deepseek/deepseek-chat"
    ));
}

#[test]
fn codex_offers_an_openai_key_instead_of_a_sign_in() {
    let mut rt = scene("mac-assistant-setup-agent");
    tap(&mut rt, "codex-use-key");
    rt.state.chats.settings.agents.codex_key = Some(TextEdit::new("sk-openai-123"));
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "codex-use");
    assert!(
        rt.state
            .chats
            .jobs
            .asked
            .contains(&Job::CodexKey("sk-openai-123".into()))
    );
}

#[test]
fn the_display_row_shows_only_with_two_displays() {
    let mut rt = scene("mac-assistant-setup-desktop");
    assert!(rt.find("agent-display").is_some(), "two displays in scenes");
    rt.state.chats.settings.displays.truncate(1);
    rt.invalidate();
    settle(&mut rt);
    assert!(rt.find("agent-display").is_none());
}

#[test]
fn the_privacy_page_sets_autonomy_and_blocked_apps() {
    let mut rt = scene("mac-assistant-setup-privacy");
    assert!(shows(&rt, "Dictation stays on your Mac."));
    assert_eq!(
        rt.state.settings.assistant.autonomy,
        crate::assistant::autonomy::Autonomy::Smart
    );
    rt.state.chats.settings.blocked = TextEdit::new("Banking");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "add-blocked-app");
    assert!(
        rt.state
            .settings
            .assistant
            .blocked_apps
            .contains(&"Banking".to_owned())
    );
}

#[test]
fn schedules_list_toggle_run_and_add_by_hand() {
    let mut rt = chats();
    assert!(shows(&rt, "Summarize my inbox and flag what needs me"));
    tap(&mut rt, "new-schedule");
    let sheet = rt.state.chats.new_schedule.as_mut().unwrap();
    sheet.text = TextEdit::new("Back up my notes");
    sheet.time = TextEdit::new("25:00");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "save-schedule");
    assert!(shows(&rt, "Use a time like 8:00 or 17:30"));
    rt.state.chats.new_schedule.as_mut().unwrap().time = TextEdit::new("21:15");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "save-schedule");
    assert!(rt.state.chats.new_schedule.is_none());
    assert_eq!(rt.state.chats.schedules.len(), 3);
    assert_eq!(rt.state.chats.schedules[2].minute_of_day, 21 * 60 + 15);
    rt.update(|ui, _| ui.chats.run_schedule("inbox"));
    settle(&mut rt);
    let sent = rt.state.chats.store().unwrap().submitted.last().unwrap();
    assert_eq!(sent.text, "Summarize my inbox and flag what needs me");
    assert_eq!(sent.conversation, None, "a new conversation");
}

#[test]
fn page_names_round_trip_for_the_saved_progress() {
    for page in Page::shown().iter().copied() {
        assert_eq!(Page::named(page.name()), Some(page));
    }
    assert_eq!(Page::named("permissions"), None);
}

#[test]
fn the_merged_flow_says_it_continues_and_new_in_hex_says_so() {
    let rt = scene("mac-assistant-setup-merged");
    assert!(shows(&rt, "Now, your assistant"));
    let rt = scene("mac-assistant-setup-new");
    assert!(shows(&rt, "New in Agent-Desk: your assistant"));
    let rt = scene("mac-assistant-setup-resumed");
    assert!(shows(
        &rt,
        "Agent-Desk restarted. Checking again where you left off…"
    ));
}

#[test]
fn checks_run_on_arrival_and_finish_waits_for_answers_and_the_desktop_tools() {
    let mut rt = onboarding();
    rt.update(|ui, _| super::setup::go(ui, Page::Checks));
    settle(&mut rt);
    assert!(
        rt.state
            .chats
            .jobs
            .asked
            .iter()
            .any(|job| matches!(job, Job::SelfTests(..)))
    );
    let backend = AssistantBackend::ClaudeCode;
    let ok = |check| CheckResult {
        check,
        state: CheckState::Ok,
        detail: "fine".into(),
        fix: None,
        took: std::time::Duration::ZERO,
    };
    let finishable = |rt: &Runtime<Ui>| {
        super::setup::can_finish(&rt.state, rt.state.chats.setup.as_ref().unwrap())
    };
    answer(&mut rt, Done::Check(ok(Check::Answers(backend))));
    assert!(!finishable(&rt), "answering alone isn't enough");
    answer(
        &mut rt,
        Done::Check(ok(Check::DesktopThroughAgent(backend))),
    );
    assert!(finishable(&rt));
    assert!(
        !super::setup::gestures_work(&rt.state),
        "not on the Try page"
    );
    rt.update(|ui, _| super::setup::go(ui, Page::Try));
    settle(&mut rt);
    assert!(
        super::setup::gestures_work(&rt.state),
        "the gestures the Try page asks for answer before Finish"
    );
    tap(&mut rt, "finish");
    assert!(rt.state.settings.assistant.setup_done);
    assert!(rt.state.chats.setup.is_none());
    assert_eq!(rt.state.pane, Pane::Chats);
}

#[test]
fn the_try_page_checks_each_gesture_live() {
    let mut rt = onboarding();
    rt.update(|ui, _| super::setup::go(ui, Page::Try));
    settle(&mut rt);
    let done = |rt: &Runtime<Ui>| {
        let flow = rt.state.chats.setup.as_ref().unwrap();
        super::setup::page_done(&rt.state, flow, Page::Try)
    };
    assert!(shows(&rt, "Listening"));
    // Hold: a question about the window in front, answered.
    rt.state.chats.deliver(Event::Started {
        conversation: "q".into(),
        origin: Origin::ActionShortcut,
        question: "What's on this page?".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    rt.state.chats.deliver(Event::Answer {
        conversation: "q".into(),
        origin: Origin::ActionShortcut,
        text: "Agent-Desk's release notes.".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Answered"));
    assert!(!done(&rt), "the background gesture is still to try");
    // A Chat task isn't the gesture; a Background one is.
    for origin in [Origin::Chat, Origin::Background] {
        rt.state.chats.deliver(Event::Queued {
            conversation: format!("{origin:?}"),
            origin,
            question: "Find a free hour next week".into(),
        });
    }
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Started"));
    assert!(done(&rt), "double-tap is optional");
    // The HUD reports the double-tap.
    super::tasks_opened();
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "Opened"));
}

#[test]
fn the_shortcut_page_waits_for_a_free_shortcut_and_a_real_press() {
    use super::setup::ProbeState;
    use super::shortcut::{Conflict, ConflictFix, Probe};
    use crate::app_settings::HotkeySlot;
    let mut rt = onboarding();
    rt.update(|ui, _| super::setup::go(ui, Page::Shortcut));
    settle(&mut rt);
    assert!(
        rt.state
            .chats
            .jobs
            .asked
            .iter()
            .any(|job| matches!(job, Job::Conflicts(_)))
    );
    assert!(
        shows(&rt, "Press and release it now"),
        "the Action test listens"
    );
    let done = |rt: &Runtime<Ui>| {
        let flow = rt.state.chats.setup.as_ref().unwrap();
        super::setup::page_done(&rt.state, flow, Page::Shortcut)
    };
    let siri = Conflict {
        name: "Siri".into(),
        description: "Press Right Command twice talks to Siri.".into(),
        taken: true,
        fix: ConflictFix::SiriSettings,
    };
    answer(&mut rt, Done::Conflicts(vec![(HotkeySlot::Action, siri)]));
    assert!(shows(&rt, "Siri takes it"));
    let probed = |rt: &mut Runtime<Ui>, found: Probe| {
        let flow = rt.state.chats.setup.as_mut().unwrap();
        flow.probes = vec![(HotkeySlot::Action, ProbeState::Done(found))];
        rt.invalidate();
        settle(rt);
    };
    probed(&mut rt, Probe::Swallowed);
    assert!(shows(&rt, "Siri took the keys before Agent-Desk could"));
    assert!(!done(&rt));
    probed(&mut rt, Probe::Works(std::time::Duration::from_millis(400)));
    assert!(!done(&rt), "a taken shortcut keeps the page unfinished");
    answer(&mut rt, Done::Conflicts(Vec::new()));
    assert!(done(&rt));
    assert!(shows(&rt, "Agent-Desk received it (held 0.4 s)"));
}

#[test]
fn onboarding_polish_back_leaves_room_for_macos_window_controls() {
    let rt = scene("mac-assistant-setup-desktop");
    let back = rt
        .elements()
        .into_iter()
        .find(|element| element.path.ends_with("/back"))
        .expect("Back button");
    assert!(
        back.rect[0] >= 88.0,
        "Back overlaps the native window controls: {:?}",
        back.rect
    );
}

#[test]
fn onboarding_polish_sign_in_chooses_a_site_before_opening_a_browser() {
    let mut rt = scene("mac-assistant-setup-browser");
    tap(&mut rt, "sign-in-sites");
    assert!(
        rt.find("sign-in-website").is_some(),
        "choose a website first"
    );
    assert!(
        !rt.state
            .chats
            .jobs
            .asked
            .iter()
            .any(|job| matches!(job, Job::SignIn(_))),
        "opening the editor must not navigate"
    );
}

#[test]
fn onboarding_polish_screen_recording_restart_is_offered() {
    let mut rt = scene("mac-assistant-setup-desktop");
    rt.state.platform.scrawl = Some(super::super::platform::ScrawlView {
        screen_needs_restart: true,
        ..Default::default()
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(
        rt.find("restart").is_some(),
        "a granted screen permission needs Restart, not another Allow"
    );
}

#[test]
fn onboarding_polish_refreshes_failed_checks_when_permissions_change() {
    let mut rt = scene("mac-assistant-setup-checks");
    {
        let flow = rt.state.chats.setup.as_mut().unwrap();
        flow.checks_running = false;
        flow.permissions = Some(Permissions {
            accessibility: false,
            screen_recording: false,
        });
    }
    let before = rt.state.chats.jobs.asked.len();
    answer(
        &mut rt,
        Done::Permissions(Permissions {
            accessibility: true,
            screen_recording: true,
        }),
    );
    assert!(
        rt.state.chats.jobs.asked[before..]
            .iter()
            .any(|job| matches!(job, Job::SelfTests(..))),
        "the Checks page must recheck a changed grant"
    );
}

#[test]
fn a_selected_sign_in_site_opens_once_and_failed_saves_can_be_retried() {
    let mut rt = scene("mac-assistant-setup-browser");
    tap(&mut rt, "sign-in-sites");
    rt.state.chats.settings.site_address = TextEdit::new("mail.google.com");
    super::site_sign_in::launch(&mut rt.state);
    super::site_sign_in::launch(&mut rt.state);
    let opens: Vec<_> = rt
        .state
        .chats
        .jobs
        .asked
        .iter()
        .filter_map(|job| match job {
            Job::SignIn(url) => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(opens, [Some("https://mail.google.com/".to_owned())]);
    answer(&mut rt, Done::SignIn(true, Ok(())));
    super::site_sign_in::save(&mut rt.state);
    answer(
        &mut rt,
        Done::SignIn(false, Err("fixture save failed".into())),
    );
    assert!(
        rt.state.chats.settings.signing_in,
        "a failed close leaves the browser open"
    );
    assert!(!rt.state.chats.settings.sign_in_busy, "Save can be retried");
    assert!(rt.state.chats.settings.sign_in_editor);
}

#[test]
fn stale_assistant_grants_use_the_running_apps_permission_repair() {
    use super::super::platform::{PlatformHost, PlatformRequest};
    use crate::desktop::onboarding::{PermissionAction, PermissionKind};
    let mut rt = scene("mac-assistant-setup-desktop");
    rt.state.platform.note_stale(PermissionKind::Accessibility);
    super::setup::allow(&mut rt.state, Grant::DesktopAccessibility);
    let PlatformHost::Fixture(sent) = &rt.state.host else {
        panic!("fixture host");
    };
    assert!(sent.contains(&PlatformRequest::Permission(
        PermissionAction::ResetAndPrompt(PermissionKind::Accessibility)
    )));
    assert!(!rt.state.chats.jobs.asked.iter().any(|job| matches!(
        job,
        Job::Fix(crate::assistant::backends::Fix::OpenSettings(_))
    )));
}

mod repair {
    use super::*;
    use crate::desktop::setup_gaps::{Gap, SETUP_VERSION};

    fn gaps(rt: &mut Runtime<Ui>, gaps: Vec<Gap>) {
        answer(rt, Done::Gaps(gaps));
    }

    /// The stage card's Fix all…: the person asks for the guide.
    fn fix_all(rt: &mut Runtime<Ui>) {
        rt.update(|ui, tx| super::super::repair(ui, true, tx));
        settle(rt);
    }

    #[test]
    fn the_repair_check_never_switches_the_window() {
        let mut rt = chats();
        let pane = rt.state.pane;
        gaps(&mut rt, vec![Gap::Shortcut, Gap::ScreenRecording]);
        assert!(rt.state.chats.setup.is_none(), "no guide by itself");
        assert_eq!(rt.state.pane, pane, "the window shows what it showed");
        // The same again in five minutes: still nothing.
        gaps(&mut rt, vec![Gap::Shortcut, Gap::ScreenRecording]);
        assert!(rt.state.chats.setup.is_none());
        // Fix all… on the stage's card opens it.
        fix_all(&mut rt);
        assert_eq!(page(&rt), Page::Desktop);
        // Later on the card puts it off until Agent-Desk starts again.
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::ScreenRecording]);
        rt.update(|ui, tx| super::super::repair(ui, false, tx));
        assert!(rt.state.chats.repair.snoozed);
        assert!(rt.state.chats.setup.is_none());
    }

    fn repair_pages(rt: &Runtime<Ui>) -> Vec<Page> {
        rt.state
            .chats
            .setup
            .as_ref()
            .and_then(|flow| flow.repair.as_ref())
            .map(|repair| repair.pages.clone())
            .unwrap_or_default()
    }

    #[test]
    fn nothing_missing_shows_nothing() {
        let mut rt = chats();
        gaps(&mut rt, Vec::new());
        assert!(rt.state.chats.setup.is_none());
    }

    #[test]
    fn what_went_missing_brings_back_only_its_pages_and_says_why() {
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::Shortcut, Gap::ScreenRecording]);
        fix_all(&mut rt);
        assert_eq!(
            page(&rt),
            Page::Desktop,
            "starts at the first page that fixes one"
        );
        assert_eq!(repair_pages(&rt), [Page::Desktop, Page::Shortcut]);
        assert!(shows(&rt, "Agent-Desk needs your help for a moment"));
        assert!(shows(
            &rt,
            "Agent-Desk can't see the screen: Screen Recording is off"
        ));
        assert!(shows(&rt, "Another app took the assistant's shortcut"));
        assert!(
            rt.find("back").is_none(),
            "nothing before the guide's first page"
        );
    }

    #[test]
    fn later_puts_the_guide_off_until_hex_starts_again_but_a_hold_opens_it() {
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::ScreenRecording]);
        fix_all(&mut rt);
        tap(&mut rt, "later");
        assert!(rt.state.chats.setup.is_none());
        assert!(rt.state.chats.repair.snoozed);
        // The next check finds it still missing: no window this launch.
        gaps(&mut rt, vec![Gap::ScreenRecording]);
        assert!(rt.state.chats.setup.is_none());
        // A hold of the Action shortcut brings it back.
        rt.update(|ui, tx| super::super::open_setup_from_hold(ui, tx));
        settle(&mut rt);
        assert_eq!(page(&rt), Page::Desktop);
    }

    #[test]
    fn closing_the_guide_puts_it_off_too() {
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::ScreenRecording]);
        fix_all(&mut rt);
        tap(&mut rt, "close-setup");
        assert!(rt.state.chats.repair.snoozed);
    }

    #[test]
    fn a_missing_voice_downloads_again_without_a_page() {
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::HexVoice]);
        assert!(rt.state.chats.setup.is_none());
        assert!(rt.state.chats.jobs.asked.contains(&Job::DownloadVoice));
    }

    #[test]
    fn an_essential_gap_is_what_a_hold_says() {
        let mut rt = chats();
        gaps(&mut rt, vec![Gap::ScreenRecording, Gap::AgentSignIn]);
        let said =
            crate::desktop::setup_gaps::essential(&rt.state.chats.repair.gaps).map(Gap::message);
        assert_eq!(said, Some("The assistant's agent is signed out"));
        gaps(&mut rt, Vec::new());
        assert!(crate::desktop::setup_gaps::essential(&rt.state.chats.repair.gaps).is_none());
    }

    #[test]
    fn a_page_that_works_moves_on_by_itself_and_the_last_ends_the_guide() {
        let mut rt = chats();
        rt.state.settings.assistant.setup_version = 0;
        // The welcome page always works: a stand-in for a step a release
        // added that's already done.
        gaps(&mut rt, vec![Gap::NewSteps(vec!["welcome"])]);
        fix_all(&mut rt);
        assert_eq!(page(&rt), Page::Welcome);
        settle(&mut rt);
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        rt.invalidate();
        settle(&mut rt);
        assert!(rt.state.chats.setup.is_none(), "the guide ended by itself");
        assert_eq!(rt.state.settings.assistant.setup_version, SETUP_VERSION);
        assert!(!rt.state.chats.repair.snoozed, "finished, not put off");
    }
}

/// One stream: every stored conversation, oldest first by when it was
/// asked, with no list beside it.
#[test]
fn every_conversation_shows_in_one_stream_by_when_it_was_asked() {
    let pane = pane_with(Vec::new());
    assert_eq!(pane.stream.len(), pane.list.len());
    let asked: Vec<u64> = pane.stream.iter().map(|c| c.created_ms).collect();
    let mut sorted = asked.clone();
    sorted.sort_unstable();
    assert_eq!(asked, sorted, "oldest first");
    let rt = chats();
    assert!(rt.find("chats-list").is_none(), "no list of chats");
    assert!(rt.find("chats-search").is_none());
    for id in pane.list.iter().map(|row| row.id.as_str()) {
        assert!(
            rt.find(&format!("entry-{id}")).is_some(),
            "{id} is in the stream"
        );
    }
}

/// Following new activity highlights it; only a click points the
/// composer at an entry, and the clear button points it back.
#[test]
fn only_a_click_points_the_composer_at_an_entry() {
    let mut rt = chats();
    rt.update(|ui, _| ui.chats.auto_select());
    settle(&mut rt);
    assert!(rt.state.chats.selected.is_some());
    assert_eq!(rt.state.chats.target, None, "following never targets");
    assert!(rt.find("composer-target").is_none());
    tap(&mut rt, "chat-reply-to-maya");
    assert_eq!(rt.state.chats.target.as_deref(), Some("reply-to-maya"));
    assert!(rt.find("composer-target").is_some());
    tap(&mut rt, "clear-target");
    assert_eq!(rt.state.chats.target, None);
    rt.state.chats.composer = TextEdit::new("What did the Maya task do?");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-send");
    let store = rt.state.chats.store().unwrap();
    let sent = store.submitted.last().unwrap();
    assert_eq!(sent.conversation, None, "a request of its own");
    assert!(
        rt.state.chats.turns["reply-to-maya"].steered.is_empty(),
        "the running task isn't steered"
    );
}

#[test]
fn a_job_answered_by_voice_goes_on_without_doubling_or_its_old_question() {
    let mut rt = chats();
    let id = "shopping";
    rt.state.chats.apply(&Event::Started {
        conversation: id.into(),
        origin: Origin::ActionShortcut,
        question: "Open Notes and write a shopping list".into(),
    });
    rt.state.chats.apply(&Event::AnswerDelta {
        conversation: id.into(),
        text: "On it.".into(),
    });
    rt.state.chats.apply(&Event::Question {
        conversation: id.into(),
        text: "What goes on it?".into(),
    });
    assert!(
        rt.state.chats.in_flight(id).unwrap().answer.is_empty(),
        "stored with the question"
    );
    // Answered by voice: the service starts it again with the answer.
    rt.state.chats.apply(&Event::Started {
        conversation: id.into(),
        origin: Origin::ActionShortcut,
        question: "Apples, pears and zucchini".into(),
    });
    assert!(rt.state.chats.question(id).is_none(), "its box goes");
    rt.state.chats.apply(&Event::AnswerDelta {
        conversation: id.into(),
        text: "Done.".into(),
    });
    assert_eq!(rt.state.chats.in_flight(id).unwrap().answer, "Done.");
}

/// A request that only stopped (a quick question moved to the background
/// settles that way) is a quiet state on its entry: never "Failed", never a
/// red error.
#[test]
fn a_stop_is_a_quiet_state_not_an_error() {
    let mut rt = chats();
    let failed = |rt: &Runtime<Ui>| texts(rt).iter().filter(|text| *text == "Failed").count();
    let before = failed(&rt);
    rt.state.chats.deliver(Event::Started {
        conversation: "quick".into(),
        origin: Origin::ActionShortcut,
        question: "Open Mail".into(),
    });
    rt.state.chats.deliver(Event::Failed {
        conversation: "quick".into(),
        message: "Stopped.".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert_eq!(failed(&rt), before, "{:?}", texts(&rt));
    assert!(!shows(&rt, "Stopped."), "no error row");
    assert!(shows(&rt, "Stopped"), "a quiet line says it stopped");
}
