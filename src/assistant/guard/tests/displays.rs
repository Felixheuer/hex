//! The guard's tests for displays, with this Mac's layout: the built-in
//! display (1, primary, 1800×1169 at 0,0) and an M27Q (5, 2560×1440 at
//! -604,-1440, above and left of it), where the recorded windows are.

use super::*;
use crate::assistant::computer;

fn display(id: &str, frame: (f64, f64, f64, f64), focus: bool) -> super::super::screen::Display {
    computer::Display {
        id: id.into(),
        name: if id == "5" {
            "M27Q".into()
        } else {
            "Built-in Retina Display".into()
        },
        frame,
        visible_frame: frame,
        scale: 2.0,
        is_primary: frame.0 == 0.0 && frame.1 == 0.0,
        has_focus: focus,
    }
}

/// The person on the built-in display.
fn this_mac(fake: &Fake) {
    fake.state().displays = vec![
        display("1", (0.0, 0.0, 1800.0, 1169.0), true),
        display("5", (-604.0, -1440.0, 2560.0, 1440.0), false),
    ];
}

fn moves(fake: &Fake) -> Vec<Map<String, Value>> {
    fake.state()
        .calls
        .iter()
        .filter(|(tool, _)| tool == "desktop_move_window")
        .map(|(_, args)| args.clone())
        .collect()
}

fn textedit_window(id: &str, frame: (f64, f64, f64, f64)) -> Value {
    json!({
        "app_name": "TextEdit", "id": id, "is_focused": false, "pid": 900,
        "title": "Untitled", "visible": true,
        "bounds": {"x": frame.0, "y": frame.1, "width": frame.2, "height": frame.3},
    })
}

#[test]
fn hex_displays_tells_the_layout_and_both_displays() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    assert!(
        guard
            .tools()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "hex_displays")
    );
    let data = child::data(&guard.call("hex_displays", Map::new())).unwrap();
    assert_eq!(data["person_display"], "1");
    assert_eq!(data["agents_display"], "5");
    assert_eq!(
        data["displays"][1]["frame"],
        json!([-604.0, -1440.0, 2560.0, 1440.0])
    );
    assert!(data["layout"].as_str().unwrap().contains(
        "M27Q (display 5, 2560×1440 points at (-604, -1440), scale 2) is above and left of it"
    ));
    assert!(fake.forwarded().is_empty(), "read in Hex");
}

#[test]
fn a_background_tasks_new_window_moves_to_the_agents_display() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    // It opens on the person's display.
    fake.state().on_launch = Some(textedit_window("w-500", (100.0, 100.0, 500.0, 400.0)));
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    let moved = moves(&fake);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0]["window_id"], "w-500");
    // Cascaded on the M27Q, in negative coordinates (one lease before it).
    assert_eq!(moved[0]["x"], -532.0);
    assert_eq!(moved[0]["y"], -1368.0);
    // One that opens on the agents' display already stays.
    fake.state().on_launch = Some(textedit_window("w-501", (-400.0, -1000.0, 500.0, 400.0)));
    guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    assert_eq!(moves(&fake).len(), 1);
    assert_eq!(scratch.store().holder("w-501").unwrap().task, "bg");

    // The agents' display is unplugged: its windows come back to the main
    // display, and the task is told.
    fake.state().displays.truncate(1);
    let next = guard.call("desktop_list_windows", Map::new());
    let moved = moves(&fake);
    assert_eq!(moved.len(), 3);
    for back in &moved[1..] {
        let (x, y) = (back["x"].as_f64().unwrap(), back["y"].as_f64().unwrap());
        assert!(
            (0.0..1800.0).contains(&x) && (0.0..1169.0).contains(&y),
            "{back:?}"
        );
    }
    assert!(
        notes(&next).iter().any(|note| note.contains("is gone")),
        "{next}"
    );
}

#[test]
fn with_one_display_or_in_the_here_lane_windows_stay_put() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    fake.state().displays = vec![display("1", (0.0, 0.0, 1800.0, 1169.0), true)];
    fake.state().on_launch = Some(textedit_window("w-500", (100.0, 100.0, 500.0, 400.0)));
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    assert!(moves(&fake).is_empty());
    assert_eq!(scratch.store().holder("w-500").unwrap().task, "bg");
    // A chosen display that's there takes them, even the person's own.
    let fake = Fake::recorded();
    this_mac(&fake);
    fake.state().on_launch = Some(textedit_window("w-600", (-400.0, -1000.0, 500.0, 400.0)));
    let scratch = Scratch::new();
    let mut guard = Guard::new(
        Config {
            task: "bg".into(),
            lane: Lane::Background,
            here_window: None,
            physical_input: false,
            blocked_apps: Vec::new(),
            agent_display: Some("Built-in Retina Display".into()),
        },
        scratch.store(),
        std::process::id(),
        fake.clone(),
        Box::new(fake.clone()),
    );
    guard.fresh_for = Duration::ZERO;
    guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    let moved = moves(&fake);
    assert_eq!(moved.len(), 1);
    assert!(moved[0]["x"].as_f64().unwrap() >= 0.0);
}

#[test]
fn points_in_negative_coordinates_find_their_window() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    let mut guard = background(&scratch, &fake, "bg", true);
    fake.state()
        .replies
        .insert("desktop_mouse_click".into(), ok("mouse-click", json!({})));
    // Citrix's window fills the M27Q (-604,-1440 2560×1440).
    guard.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    let inside = guard.call("desktop_mouse_click", args(json!({"xy": "-300,-700"})));
    assert_eq!(error_code(&inside), None, "{inside}");
    // Left of the built-in display, below the M27Q: no window there.
    let nowhere = guard.call("desktop_mouse_click", args(json!({"xy": "-300,700"})));
    assert_eq!(error_code(&nowhere).as_deref(), Some("NOT_YOUR_WINDOW"));
    // Across the seam on the built-in display: the person's window.
    let theirs = guard.call("desktop_mouse_click", args(json!({"xy": "500,200"})));
    assert_eq!(error_code(&theirs).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert_eq!(fake.forwarded(), ["desktop_mouse_click"]);
}
