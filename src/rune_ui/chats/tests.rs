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
fn agent_mode_and_the_task_strip_reach_the_service() {
    let mut rt = chats();
    assert!(rt.state.chats.agent_mode);
    tap(&mut rt, "agent-mode");
    assert!(!rt.state.chats.store().unwrap().agent_mode);
    tap(&mut rt, "stop-task");
    assert_eq!(
        rt.state.chats.store().unwrap().cancelled,
        vec!["reply-to-maya".to_owned()]
    );
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
    assert_eq!(page(&rt), Page::Permissions);
    // Screen Recording unread: Later moves on and remembers.
    tap(&mut rt, "later");
    let flow = rt.state.chats.setup.as_ref().unwrap();
    assert_eq!(flow.page, Page::Desktop);
    assert_eq!(flow.later, vec![Page::Permissions]);
}

#[test]
fn a_permission_waits_after_allow_until_macos_reports_it() {
    let mut rt = onboarding();
    rt.state.platform.permissions.microphone = crate::desktop::onboarding::PermissionState::Ready;
    rt.state.platform.permissions.accessibility =
        crate::desktop::onboarding::PermissionState::Ready;
    rt.update(|ui, _| super::setup::go(ui, Page::Permissions));
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
    assert!(super::setup::page_done(
        &rt.state,
        rt.state.chats.setup.as_ref().unwrap(),
        Page::Permissions
    ));
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
fn the_try_page_shows_the_real_question_and_its_answer() {
    let mut rt = onboarding();
    rt.update(|ui, _| super::setup::go(ui, Page::Try));
    settle(&mut rt);
    assert!(shows(&rt, "Waiting for your question"));
    rt.state.chats.deliver(Event::Started {
        conversation: "q".into(),
        origin: Origin::ActionShortcut,
        question: "What time is it in Tokyo?".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "What time is it in Tokyo?"));
    rt.state.chats.deliver(Event::Answer {
        conversation: "q".into(),
        origin: Origin::ActionShortcut,
        text: "It's 9:41 PM.".into(),
    });
    rt.invalidate();
    settle(&mut rt);
    assert!(shows(&rt, "It's 9:41 PM."));
    assert!(shows(&rt, "Answered"));
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
