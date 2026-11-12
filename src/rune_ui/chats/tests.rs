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

fn tap(rt: &mut Runtime<Ui>, key: &str) {
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
fn chats_joins_the_sidebar_only_while_the_lab_is_on() {
    let mut rt = chats();
    assert!(rt.find("nav-chats").is_some());
    rt.state.settings.labs.assistant = false;
    rt.invalidate();
    settle(&mut rt);
    assert!(rt.find("nav-chats").is_none(), "Chats left with its Lab");
    assert_eq!(
        rt.state.pane,
        Pane::General,
        "the window leaves for General"
    );
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
    assert!(rt.find("task0").is_some());
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
fn shift_return_keeps_typing_and_new_chat_starts_a_conversation() {
    let mut rt = chats();
    tap(&mut rt, "new-chat");
    assert_eq!(rt.state.chats.selected, None);
    assert!(shows(&rt, "What can I help with?"));
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
    tap(&mut rt, "new-chat");
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
    assert!(shows(&rt, "Interrupted: Hex quit while it ran"));
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
    assert!(!rt.state.chats.holds.contains_key("reply-to-maya"), "retrying");
    assert_eq!(
        rt.state.chats.holds.get("coffee"),
        Some(&super::Hold::Paused)
    );
    rt.state.chats.deliver(Event::Resumed {
        conversation: "coffee".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(!rt.state.chats.holds.contains_key("coffee"));
}

#[test]
fn an_interrupted_conversation_offers_resume() {
    let mut rt = scene("mac-chats-interrupted");
    assert!(shows(&rt, "This task stopped when Hex quit"));
    tap(&mut rt, "resume");
    assert_eq!(
        rt.state.chats.store().unwrap().resumed,
        vec!["sort-downloads".to_owned()]
    );
    assert!(rt.state.chats.running("sort-downloads"));
    assert!(rt.find("resume").is_none(), "the banner leaves while it runs");
}

#[test]
fn the_composer_sends_here_with_the_window_in_front_or_background() {
    let mut rt = chats();
    tap(&mut rt, "new-chat");
    rt.state.chats.composer = TextEdit::new("Summarize this page");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-send");
    tap(&mut rt, "new-chat");
    tap(&mut rt, "lane-here");
    rt.state.chats.composer = TextEdit::new("What's this error?");
    rt.invalidate();
    settle(&mut rt);
    tap(&mut rt, "composer-send");
    let store = rt.state.chats.store().unwrap();
    assert_eq!(store.submitted[0].origin, Origin::Chat, "Background by default");
    assert!(store.submitted[0].context.window.is_none());
    assert_eq!(store.submitted[1].origin, Origin::ActionShortcut);
    assert_eq!(
        store.submitted[1]
            .context
            .window
            .as_ref()
            .map(|window| window.app.as_str()),
        Some("Safari"),
        "Here names the window in front of Hex"
    );
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
    assert_eq!(turn.steered, vec!["Also ask about the projector".to_owned()]);
    assert_eq!(turn.steps.len(), 4, "the turn goes on");
    assert!(shows(&rt, "Sent while working"));
}

/// The pane over the fixture store with `tasks`, nothing selected yet.
fn pane_with(tasks: Vec<crate::assistant::TaskStatus>) -> super::ChatsPane {
    let mut store = super::fixture::store(crate::history::now_ms());
    store.tasks = tasks;
    super::ChatsPane::fixture(store)
}

fn task(id: &str, origin: Origin, state: crate::assistant::TaskState) -> crate::assistant::TaskStatus {
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
    assert_eq!(pane.selected.as_deref(), Some("window-question"), "Here first");
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
    tap(&mut rt, "new-chat");
    assert!(shows(&rt, "Finish setting up the assistant"));
    assert!(rt.find("composer-input").is_none(), "no chat until it's ready");
    tap(&mut rt, "continue-setup");
    assert!(rt.state.chats.setup.is_some());
}

#[test]
fn a_hold_before_setup_opens_the_onboarding_and_says_so() {
    let mut rt = chats();
    rt.state.settings.assistant.setup_done = false;
    rt.update(|ui, tx| super::open_setup_from_hold(ui, tx));
    settle(&mut rt);
    assert!(rt.state.chats.setup.as_ref().unwrap().from_hold);
    assert!(shows(
        &rt,
        "Finish setting up the assistant first. Then hold the Action shortcut again."
    ));
}

#[test]
fn turning_the_lab_on_opens_the_onboarding_until_it_is_done() {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    super::fixture_scene(&mut state, super::fixture::Scene::Empty);
    state.settings.labs.assistant = false;
    state.settings.assistant.setup_done = false;
    let mut rt = runtime_for(state);
    rt.update(|ui, _| super::super::labs::flip_for_test(ui, crate::labs::Feature::Assistant));
    settle(&mut rt);
    assert!(rt.state.settings.labs.assistant);
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
    assert!(rt.state.chats.jobs.asked.iter().any(|job| matches!(
        job,
        Job::Fix(crate::assistant::backends::Fix::OpenSettings(_))
    )));
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
    assert!(shows(&rt, "Restart Hex and continue"));
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
        shows(&rt, "Grant permissions…"),
        "the fix shows at once: {:?}",
        texts(&rt)
    );
}

#[test]
fn page_names_round_trip_for_the_saved_progress() {
    for page in Page::ALL {
        assert_eq!(Page::named(page.name()), Some(page));
    }
    assert_eq!(Page::named("permissions"), None);
}

#[test]
fn the_merged_flow_says_it_continues_and_new_in_hex_says_so() {
    let rt = scene("mac-assistant-setup-merged");
    assert!(shows(&rt, "Now, your assistant"));
    let rt = scene("mac-assistant-setup-new");
    assert!(shows(&rt, "New in Hex: your assistant"));
    let rt = scene("mac-assistant-setup-resumed");
    assert!(shows(&rt, "Hex restarted. Checking again where you left off…"));
}

#[test]
fn checks_run_on_arrival_and_finish_waits_for_answers_and_agent_desktop() {
    let mut rt = onboarding();
    rt.update(|ui, _| super::setup::go(ui, Page::Checks));
    settle(&mut rt);
    assert!(
        rt.state
            .chats
            .jobs
            .asked
            .iter()
            .any(|job| matches!(job, Job::SelfTests(_)))
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
    rt.update(|ui, _| super::setup::go(ui, Page::Try));
    settle(&mut rt);
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
        text: "Hex's release notes.".into(),
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
    assert!(shows(&rt, "Siri took the keys before Hex could"));
    assert!(!done(&rt));
    probed(&mut rt, Probe::Works(std::time::Duration::from_millis(400)));
    assert!(!done(&rt), "a taken shortcut keeps the page unfinished");
    answer(&mut rt, Done::Conflicts(Vec::new()));
    assert!(done(&rt));
    assert!(shows(&rt, "Hex received it (held 0.4 s)"));
}
