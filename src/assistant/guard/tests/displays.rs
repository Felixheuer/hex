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
    // Not offered (the guard places windows itself; the message names the
    // displays), but still answered.
    assert!(
        guard
            .tools()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != "hex_displays")
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
    // Only a display the person picked takes the agents' windows.
    guard.config.agent_display = Some("M27Q".into());
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
fn a_window_a_task_takes_goes_out_of_the_persons_way_too() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    // WhatsApp and Notes already run, on the person's display.
    for (id, app) in [("w-600", "WhatsApp"), ("w-601", "Notes")] {
        let mut window = textedit_window(id, (100.0, 100.0, 500.0, 400.0));
        window["app_name"] = json!(app);
        fake.state().windows.push(window);
    }
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.config.agent_display = Some("M27Q".into());
    guard.call("desktop_claim_window", args(json!({"window_id": "w-600"})));
    assert_eq!(
        moves(&fake).len(),
        1,
        "a claimed window moves like an opened one"
    );
    assert_eq!(moves(&fake)[0]["window_id"], "w-600");
    // With full control a window becomes the task's as it acts in it, and
    // goes the same way; the window in front stays where the person is.
    let mut here = here(&scratch, &fake, FRONT, "Hex Test", false);
    here.config.front_window = true;
    here.config.agent_display = Some("M27Q".into());
    let clicked = here.call("desktop_minimize", args(json!({"window_id": "w-601"})));
    assert_eq!(error_code(&clicked), None, "{clicked}");
    assert_eq!(scratch.store().holder("w-601").unwrap().task, "here-task");
    let moved = moves(&fake);
    assert_eq!(moved.len(), 2, "{moved:?}");
    assert_eq!(moved[1]["window_id"], "w-601");
    here.call("desktop_minimize", args(json!({"window_id": FRONT})));
    assert_eq!(moves(&fake).len(), 2, "the person's own window stays");
}

#[test]
fn with_no_display_picked_a_second_display_isn_t_taken() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    fake.state().on_launch = Some(textedit_window("w-500", (100.0, 100.0, 500.0, 400.0)));
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.config.agent_display = Some("none".into());
    guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    assert!(
        moves(&fake).is_empty(),
        "parked (off in tests), not moved to the M27Q"
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
            autonomy: Default::default(),
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

#[test]
fn a_parked_window_moved_behind_keeps_where_it_came_from_until_it_goes_back() {
    use super::super::parking::{self, How};
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    this_mac(&fake);
    // A job's TextEdit window hangs off the built-in display's corner,
    // parked from (100, 100).
    fake.state()
        .windows
        .push(textedit_window("w-700", (1799.0, 1168.0, 500.0, 400.0)));
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": "w-700"})));
    parking::park(
        &scratch.store(),
        "w-700",
        "TextEdit",
        Some((100.0, 100.0, 500.0, 400.0)),
        How::OffScreen,
        "bg",
        1,
    );
    // It ignores actions off the screen: it waits behind the person's
    // windows instead, and the store says so, still knowing where it was.
    fake.state().replies.insert(
        "desktop_click".into(),
        json!({"content": [{"type": "text", "text": "{\"ok\":false}"}], "isError": true}),
    );
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": "w-700"})));
    let reference = child::data(&snapshot).unwrap()["tree"]["children"][0]["ref_id"]
        .as_str()
        .unwrap()
        .to_owned();
    guard.call("desktop_click", args(json!({"ref_id": reference})));
    guard.call("desktop_click", args(json!({"ref_id": reference})));
    let behind = moves(&fake);
    assert_eq!(behind.len(), 1, "{behind:?}");
    assert_eq!(
        (behind[0]["x"].clone(), behind[0]["y"].clone()),
        (json!(80.0), json!(80.0))
    );
    let record = parking::parked(&scratch.store(), "w-700").unwrap();
    assert_eq!(record.how, How::Behind);
    assert_eq!(record.from, Some((100.0, 100.0, 500.0, 400.0)));
    // While the job holds it, it stays; once its lease is gone, it is one
    // Hex puts back.
    assert!(parking::lapsed(&scratch.store()).is_empty());
    drop(guard);
    scratch.store().release_all("bg");
    let lapsed = parking::lapsed(&scratch.store());
    assert_eq!(lapsed.len(), 1);
    assert_eq!(lapsed[0].window_id, "w-700");
}
