//! The guard's tests for the vision path (`hex_window_shot`,
//! `hex_window_input`) and the Accessibility fallback to pid input.

use super::*;
use crate::assistant::autonomy::Autonomy;
use crate::assistant::input::post::{Button, Gesture};

fn guard_with(
    scratch: &Scratch,
    fake: &Fake,
    autonomy: Autonomy,
    observe_only: bool,
) -> Guard<Fake> {
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
            observe_only,
        },
        scratch.store(),
        std::process::id(),
        fake.clone(),
        Box::new(fake.clone()),
    );
    guard.fresh_for = Duration::ZERO;
    guard
}

fn claim(guard: &mut Guard<Fake>, id: &str) {
    let claimed = guard.call(policy::CLAIM_TOOL, args(json!({"window_id": id})));
    assert_eq!(claimed["isError"], false, "{claimed}");
}

fn data(result: &Value) -> Value {
    child::data(result).unwrap_or(Value::Null)
}

#[test]
fn the_vision_tools_are_offered_and_classed() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    let names: Vec<String> = guard
        .tools()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    assert!(names.contains(&"hex_window_shot".to_owned()));
    assert!(names.contains(&"hex_window_input".to_owned()));
    assert_eq!(
        policy::kind("hex_window_shot", &Map::new()),
        Kind::WindowShot
    );
    assert!(policy::kind("hex_window_input", &Map::new()).acts());
    assert_eq!(
        super::super::screen_work("mcp__agent-desktop__hex_window_input"),
        Some(true)
    );
    assert_eq!(super::super::screen_work("hex_window_shot"), Some(false));
}

#[test]
fn input_goes_only_to_the_tasks_own_windows() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    let click = args(json!({"window_id": SETTINGS, "action": "click", "x": 100, "y": 80}));
    let refused = guard.call("hex_window_input", click.clone());
    assert_eq!(error_code(&refused).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert!(guard.sent().is_empty());
    claim(&mut guard, SETTINGS);
    let done = guard.call("hex_window_input", click);
    assert_eq!(done["isError"], false, "{done}");
    // Nothing went through agent-desktop's pointer: it was pid input.
    assert!(!fake.forwarded().iter().any(|tool| tool.contains("mouse")));
    let sent = guard.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 18628);
    assert_eq!(
        sent[0].1,
        Gesture::Click {
            button: Button::Left,
            count: 1
        }
    );
    // A fresh look comes back with it, at the window's pixels (no shot
    // before: the display's resolution).
    assert_eq!(data(&done)["did"]["action"], "click");
    // The window in front is the person's: never.
    let front = guard.call(
        "hex_window_input",
        args(json!({"window_id": FRONT, "action": "key", "text": "return"})),
    );
    assert!(front["isError"] == true, "{front}");
}

#[test]
fn spots_outside_the_window_and_bad_actions_are_refused() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    claim(&mut guard, SETTINGS);
    let shot = guard.call("hex_window_shot", args(json!({"window_id": SETTINGS})));
    assert_eq!(shot["isError"], false, "{shot}");
    // Shots are at the window's pixels; 825×949 points at 2× (no display
    // known in the fake: Retina assumed).
    assert_eq!(data(&shot)["image_width"], 1650);
    let outside = guard.call(
        "hex_window_input",
        args(json!({"window_id": SETTINGS, "action": "click", "x": 5000, "y": 10})),
    );
    assert_eq!(error_code(&outside).as_deref(), Some("OUT_OF_WINDOW"));
    let unknown = guard.call(
        "hex_window_input",
        args(json!({"window_id": SETTINGS, "action": "wiggle"})),
    );
    assert_eq!(error_code(&unknown).as_deref(), Some("INVALID_ARGS"));
    let no_spot = guard.call(
        "hex_window_input",
        args(json!({"window_id": SETTINGS, "action": "click"})),
    );
    assert_eq!(error_code(&no_spot).as_deref(), Some("INVALID_ARGS"));
    assert!(guard.sent().is_empty());
}

#[test]
fn a_click_on_send_needs_a_yes_unless_yolo() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let click = args(json!({"window_id": SETTINGS, "action": "click", "x": 100, "y": 80}));
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    claim(&mut guard, SETTINGS);
    guard.set_spot_label("Senden");
    let refused = guard.call("hex_window_input", click.clone());
    assert_eq!(
        error_code(&refused).as_deref(),
        Some("NEEDS_CONFIRMATION"),
        "{refused}"
    );
    assert!(guard.sent().is_empty());
    // An ordinary control goes ahead.
    guard.set_spot_label("Allgemein");
    let done = guard.call("hex_window_input", click.clone());
    assert_eq!(done["isError"], false, "{done}");
    drop(guard);
    // YOLO: no confirmation.
    let scratch = Scratch::new();
    let mut yolo = guard_with(&scratch, &fake, Autonomy::Yolo, false);
    claim(&mut yolo, SETTINGS);
    yolo.set_spot_label("Senden");
    let done = yolo.call("hex_window_input", click);
    assert_eq!(done["isError"], false, "{done}");
    assert_eq!(yolo.sent().len(), 1);
}

#[test]
fn a_check_may_look_but_never_send_input() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, true);
    let input = guard.call(
        "hex_window_input",
        args(json!({"window_id": SETTINGS, "action": "type", "text": "x"})),
    );
    assert_eq!(error_code(&input).as_deref(), Some("POLICY_DENIED"));
    assert!(guard.sent().is_empty());
}

#[test]
fn an_accessibility_failure_is_retried_then_done_by_pid_input() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    claim(&mut guard, SETTINGS);
    // The snapshot names where its button is.
    let tree = json!({
        "app": "System\u{ad}einstellungen",
        "snapshot_id": "sax",
        "window": {"id": SETTINGS, "title": ""},
        "tree": {"role": "window", "children": [
            {"role": "button", "name": "Fertig", "ref_id": "@sax:e1",
             "bounds": {"x": 400.0, "y": 120.0, "width": 80.0, "height": 24.0}},
        ]},
    });
    let failed = {
        let envelope = json!({"version": "2.6", "ok": false, "command": "click",
            "error": {"code": "ACTION_FAILED", "message": "AXPress failed: kAXErrorCannotComplete (-25204)"}});
        json!({"content": [{"type": "text", "text": envelope.to_string()}],
            "structuredContent": envelope, "isError": true})
    };
    {
        let mut state = fake.state();
        state
            .replies
            .insert("desktop_snapshot".into(), ok("snapshot", tree));
        state.replies.insert("desktop_click".into(), failed);
    }
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(snapshot["isError"], false, "{snapshot}");
    // Snapshots ask for element bounds.
    assert_eq!(fake.last_args("desktop_snapshot")["include_bounds"], true);
    let clicked = guard.call("desktop_click", args(json!({"ref_id": "@sax:e1"})));
    assert_eq!(clicked["isError"], false, "{clicked}");
    assert_eq!(data(&clicked)["fallback"], "pid_input");
    // Tried twice through Accessibility, then once by pid input at the
    // button's centre (440,132 global; 103,61 in the window at 337,71).
    let tries = fake
        .forwarded()
        .iter()
        .filter(|tool| *tool == "desktop_click")
        .count();
    assert_eq!(tries, 2);
    assert_eq!(guard.sent().len(), 1);
    assert_eq!(data(&clicked)["at_points"], json!([103.0, 61.0]));
}

#[test]
fn other_failures_are_left_as_they_came() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = guard_with(&scratch, &fake, Autonomy::Smart, false);
    claim(&mut guard, SETTINGS);
    let stale = {
        let envelope = json!({"version": "2.6", "ok": false, "command": "click",
            "error": {"code": "STALE_REF", "message": "take a new snapshot"}});
        json!({"content": [{"type": "text", "text": envelope.to_string()}],
            "structuredContent": envelope, "isError": true})
    };
    fake.state().replies.insert("desktop_click".into(), stale);
    let _ = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    let snapshot_id = format!("sfake{}", fake.state().snapshots);
    let clicked = guard.call(
        "desktop_click",
        args(json!({"ref_id": format!("@{snapshot_id}:e1")})),
    );
    assert_eq!(error_code(&clicked).as_deref(), Some("STALE_REF"));
    assert!(guard.sent().is_empty());
}
