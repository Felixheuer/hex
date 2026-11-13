//! The guard's tests for consequential actions (autonomy) and secure
//! fields.

use super::super::screen::{MenuKey, ReturnTarget};
use super::*;
use crate::assistant::autonomy::{Authorized, Autonomy, Category};

const MESSAGES: &str = "w-900";

/// A window with Send, Move to Trash, OK, a password field, a composer
/// beside its own Send button, and a body far from the toolbar's.
fn compose(fake: &Fake, window: &str, app: &str) {
    let tree = json!({
        "app": app,
        "snapshot_id": "sform",
        "window": {"id": window, "title": ""},
        "tree": {"role": "window", "children": [
            {"role": "toolbar", "children": [
                {"role": "button", "name": "Send", "ref_id": "@sform:e1"},
            ]},
            {"role": "menuitem", "name": "Move to Trash", "ref_id": "@sform:e2"},
            {"role": "textfield", "name": "Password", "states": ["secure"],
             "value": "hunter2", "ref_id": "@sform:e3"},
            {"role": "button", "name": "OK", "ref_id": "@sform:e4"},
            {"role": "group", "children": [
                {"role": "textarea", "name": "Message", "ref_id": "@sform:e5"},
                {"role": "button", "name": "Senden", "ref_id": "@sform:e6"},
            ]},
            {"role": "scrollarea", "children": [
                {"role": "textarea", "name": "Body", "ref_id": "@sform:e7"},
            ]},
            {"role": "textfield", "name": "Quantity", "ref_id": "@sform:e8"},
        ]},
    });
    fake.state()
        .replies
        .insert("desktop_snapshot".into(), ok("snapshot", tree));
}

fn guard_with(scratch: &Scratch, fake: &Fake, autonomy: Autonomy) -> Guard<Fake> {
    let mut guard = Guard::new(
        Config {
            task: "bg".into(),
            lane: Lane::Background,
            here_window: None,
            physical_input: false,
            blocked_apps: Vec::new(),
            agent_display: None,
            autonomy,
            front_window: false,
            open_apps: false,
            observe_only: false,
        },
        scratch.store(),
        std::process::id(),
        fake.clone(),
        Box::new(fake.clone()),
    );
    guard.fresh_for = Duration::ZERO;
    guard
}

fn authorize(scratch: &Scratch, categories: &[Category]) {
    let authorized = Authorized {
        categories: categories.iter().copied().collect(),
        ..Authorized::default()
    };
    let dir = scratch.0.join("authorized");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("bg.json"),
        serde_json::to_vec(&authorized).unwrap(),
    )
    .unwrap();
}

fn click(guard: &mut Guard<Fake>, element: u32) -> Value {
    guard.call(
        "desktop_click",
        args(json!({"ref_id": format!("@sform:e{element}")})),
    )
}

#[test]
fn a_send_or_delete_button_needs_the_persons_yes_in_smart_mode() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    compose(&fake, SETTINGS, "Mail");
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    let send = click(&mut guard, 1);
    assert_eq!(error_code(&send).as_deref(), Some("NEEDS_CONFIRMATION"));
    let details = &child::envelope(&send).unwrap()["error"]["details"];
    assert_eq!(details["category"], "send");
    assert_eq!(details["target"], "“Send”");
    let trash = click(&mut guard, 2);
    assert_eq!(
        child::envelope(&trash).unwrap()["error"]["details"]["category"],
        "delete"
    );
    // An ordinary button goes through.
    assert_eq!(error_code(&click(&mut guard, 4)), None);
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_click"]);
    // The request named sending: Send goes; Move to Trash still waits.
    authorize(&scratch, &[Category::Send]);
    assert_eq!(error_code(&click(&mut guard, 1)), None);
    assert_eq!(
        error_code(&click(&mut guard, 2)).as_deref(),
        Some("NEEDS_CONFIRMATION")
    );
}

#[test]
fn ask_waits_for_everything_and_yolo_for_nothing() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    compose(&fake, SETTINGS, "Mail");
    authorize(&scratch, &[Category::Send, Category::Delete]);
    let mut asking = guard_with(&scratch, &fake, Autonomy::Ask);
    asking.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    asking.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(
        error_code(&click(&mut asking, 1)).as_deref(),
        Some("NEEDS_CONFIRMATION")
    );
    let mut yolo = guard_with(&scratch, &fake, Autonomy::Yolo);
    yolo.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(error_code(&click(&mut yolo, 2)), None);
}

/// Any app, named anything: what a key does is read from it.
fn chat_app(fake: &Fake, name: &str) {
    fake.state().windows.push(json!({
        "app_name": name, "id": MESSAGES, "is_focused": false, "pid": 4000,
        "title": "Max", "visible": true,
        "bounds": {"x": 100.0, "y": 100.0, "width": 600.0, "height": 500.0},
    }));
    compose(fake, MESSAGES, name);
}

#[test]
fn return_beside_a_send_control_and_a_send_shortcut_are_consequential() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    chat_app(&fake, "Some Chat");
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_claim_window", args(json!({"window_id": MESSAGES})));
    guard.call("desktop_snapshot", args(json!({"window_id": MESSAGES})));
    // The composer sits beside its "Senden" button.
    let typed = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e5", "text": "On my way\n"})),
    );
    assert_eq!(error_code(&typed).as_deref(), Some("NEEDS_CONFIRMATION"));
    let details = &child::envelope(&typed).unwrap()["error"]["details"];
    assert_eq!(details["category"], "send");
    assert_eq!(details["app"], "Some Chat");
    // A body's Return is a new line: the toolbar's Send is the window's.
    let body = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e7", "text": "Hi Max,\n\nsee you.\n"})),
    );
    assert_eq!(error_code(&body), None);
    // Return pressed in the focused composer, as Accessibility tells it.
    fake.state().returns.insert(
        4000,
        ReturnTarget {
            editable: true,
            multiline: Some(true),
            default_button: None,
            near: vec!["Emoji".into(), "Send".into()],
        },
    );
    let pressed = guard.call(
        "desktop_press",
        args(json!({"combo": "return", "window_id": MESSAGES})),
    );
    assert_eq!(error_code(&pressed).as_deref(), Some("NEEDS_CONFIRMATION"));
    // Typing without Return only drafts.
    let draft = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e5", "text": "On my way"})),
    );
    assert_eq!(error_code(&draft), None);
    assert_eq!(
        fake.forwarded(),
        ["desktop_snapshot", "desktop_type", "desktop_type"]
    );
}

#[test]
fn a_shortcut_is_what_the_apps_own_menu_says_it_is() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    chat_app(&fake, "Any Mail");
    let item = |title: &str, modifiers: &[&'static str], key: &str| MenuKey {
        title: title.into(),
        modifiers: modifiers.to_vec(),
        key: key.into(),
        enabled: true,
    };
    fake.state().menus.insert(
        4000,
        vec![
            item("Send Message", &["cmd", "shift"], "d"),
            item("Move to Trash", &["cmd"], "delete"),
            item("Select All", &["cmd"], "a"),
        ],
    );
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_claim_window", args(json!({"window_id": MESSAGES})));
    let press = |guard: &mut Guard<Fake>, combo: &str| {
        guard.call(
            "desktop_press",
            args(json!({"combo": combo, "window_id": MESSAGES})),
        )
    };
    let sent = press(&mut guard, "cmd+shift+d");
    assert_eq!(error_code(&sent).as_deref(), Some("NEEDS_CONFIRMATION"));
    assert_eq!(
        child::envelope(&sent).unwrap()["error"]["details"]["target"],
        "the cmd+shift+d shortcut (“Send Message”)"
    );
    let trashed = press(&mut guard, "Cmd+Backspace");
    assert_eq!(
        child::envelope(&trashed).unwrap()["error"]["details"]["category"],
        "delete"
    );
    assert_eq!(error_code(&press(&mut guard, "cmd+a")), None);
    // A key no menu item has: nothing to tell, so it runs.
    assert_eq!(error_code(&press(&mut guard, "cmd+k")), None);
    // The same keys mean something else in another app.
    fake.state()
        .menus
        .insert(4000, vec![item("Duplicate", &["cmd", "shift"], "d")]);
    assert_eq!(error_code(&press(&mut guard, "cmd+shift+d")), None);
    // YOLO reads no menu at all.
    let mut yolo = guard_with(&scratch, &fake, Autonomy::Yolo);
    fake.state()
        .menus
        .insert(4000, vec![item("Send Message", &["cmd", "shift"], "d")]);
    assert_eq!(error_code(&press(&mut yolo, "cmd+shift+d")), None);
}

#[test]
fn return_in_a_dialog_presses_its_default_button() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    chat_app(&fake, "Any Shop");
    fake.state().returns.insert(
        4000,
        ReturnTarget {
            editable: true,
            multiline: Some(false),
            default_button: Some("Jetzt kaufen".into()),
            near: Vec::new(),
        },
    );
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_claim_window", args(json!({"window_id": MESSAGES})));
    guard.call("desktop_snapshot", args(json!({"window_id": MESSAGES})));
    // A one-line field: Return presses the default button.
    let typed = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e8", "text": "2\n"})),
    );
    assert_eq!(
        child::envelope(&typed).unwrap()["error"]["details"]["category"],
        "pay"
    );
    // A field of many lines takes the Return as a new line.
    let body = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e7", "text": "2\n"})),
    );
    assert_eq!(error_code(&body), None);
    authorize(&scratch, &[Category::Pay]);
    let typed = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e8", "text": "2\n"})),
    );
    assert_eq!(error_code(&typed), None);
}

#[test]
fn a_password_fields_value_is_never_read() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    compose(&fake, SETTINGS, "Safari");
    let mut guard = guard_with(&scratch, &fake, Autonomy::Yolo);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert!(!snapshot.to_string().contains("hunter2"), "{snapshot}");
    let field = &child::data(&snapshot).unwrap()["tree"]["children"][2];
    assert_eq!(field["value"], json!({"redacted": true}));
    // The text block the model reads is redacted too.
    let text = snapshot["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("redacted") && !text.contains("hunter2"));
    for property in ["value", "text"] {
        let read = guard.call(
            "desktop_get",
            args(json!({"ref_id": "@sform:e3", "property": property})),
        );
        assert_eq!(error_code(&read).as_deref(), Some("POLICY_DENIED"));
    }
    let bounds = guard.call(
        "desktop_get",
        args(json!({"ref_id": "@sform:e3", "property": "bounds"})),
    );
    assert_eq!(error_code(&bounds), None);
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_get"]);
}
