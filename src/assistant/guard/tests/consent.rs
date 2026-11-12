//! The guard's tests for consequential actions (autonomy) and secure
//! fields.

use super::*;
use crate::assistant::autonomy::{Authorized, Autonomy, Category};

const MESSAGES: &str = "w-900";

/// A window with Send, Move to Trash, OK, and a password field.
fn compose(fake: &Fake, window: &str, app: &str) {
    let tree = json!({
        "app": app,
        "snapshot_id": "sform",
        "window": {"id": window, "title": ""},
        "tree": {"role": "window", "children": [
            {"role": "button", "name": "Send", "ref_id": "@sform:e1"},
            {"role": "menuitem", "name": "Move to Trash", "ref_id": "@sform:e2"},
            {"role": "textfield", "name": "Password", "states": ["secure"],
             "value": "hunter2", "ref_id": "@sform:e3"},
            {"role": "button", "name": "OK", "ref_id": "@sform:e4"},
            {"role": "textfield", "name": "Message", "ref_id": "@sform:e5"},
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

#[test]
fn return_in_a_chat_and_known_shortcuts_are_consequential_too() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    fake.state().windows.push(json!({
        "app_name": "Messages", "id": MESSAGES, "is_focused": false, "pid": 4000,
        "title": "Max", "visible": true,
        "bounds": {"x": 100.0, "y": 100.0, "width": 600.0, "height": 500.0},
    }));
    compose(&fake, MESSAGES, "Messages");
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_claim_window", args(json!({"window_id": MESSAGES})));
    guard.call("desktop_snapshot", args(json!({"window_id": MESSAGES})));
    let typed = guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e5", "text": "On my way\n"})),
    );
    assert_eq!(error_code(&typed).as_deref(), Some("NEEDS_CONFIRMATION"));
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
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_type"]);
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
