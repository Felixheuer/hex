//! The guard's tests for Cua Driver used fully: batches checked step by
//! step, scripts whose every call is the guard's, menus, the whole screen,
//! the clipboard, skills recorded and replayed, and the spot a task acted
//! at.

use super::*;
use crate::assistant::autonomy::{Authorized, Autonomy, Category};

/// A form in `window` (of `app`) with Send, OK, a name field, and frames.
fn form(fake: &Fake, window: &str, app: &str) {
    let tree = json!({
        "app": app,
        "snapshot_id": "sform",
        "window": {"id": window, "title": ""},
        "tree": {"role": "window", "children": [
            {"role": "button", "name": "Send", "ref_id": "@sform:e1",
             "bounds": {"x": 400.0, "y": 100.0, "width": 60.0, "height": 20.0}},
            {"role": "button", "name": "OK", "ref_id": "@sform:e2",
             "bounds": {"x": 700.0, "y": 950.0, "width": 40.0, "height": 20.0}},
            {"role": "textfield", "name": "Name", "ref_id": "@sform:e3"},
            // TextEdit's text area is labelled with its own text.
            {"role": "textarea", "name": "Dear Ada,", "value": "Dear Ada,", "ref_id": "@sform:e4"},
        ]},
    });
    fake.state()
        .replies
        .insert("desktop_snapshot".into(), ok("snapshot", tree));
}

fn task(scratch: &Scratch, fake: &Fake, autonomy: Autonomy) -> Guard<Fake> {
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
    guard.skills_dir = scratch.0.join("skills");
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

/// A task holding Settings, with the form read.
fn holding_settings(scratch: &Scratch, fake: &Fake, autonomy: Autonomy) -> Guard<Fake> {
    form(fake, SETTINGS, "Systemeinstellungen");
    let mut guard = task(scratch, fake, autonomy);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    guard
}

#[test]
fn a_batch_acts_only_in_its_window_and_every_step_is_checked() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    let steps = |steps: Value| args(json!({"window_id": SETTINGS, "steps": steps}));
    let ran = guard.call(
        "desktop_run_steps",
        steps(json!([
            {"action": "type", "ref_id": "@sform:e3", "text": "Ada"},
            {"action": "click", "ref_id": "@sform:e2"},
            {"action": "click", "role": "button", "name": "Apply"},
        ])),
    );
    assert_eq!(error_code(&ran), None, "{ran}");
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_run_steps"]);
    // A step can't name another window, nor use another window's ref.
    let elsewhere = guard.call(
        "desktop_run_steps",
        steps(json!([{"action": "click", "name": "OK", "app": "Finder"}])),
    );
    assert_eq!(error_code(&elsewhere).as_deref(), Some("INVALID_ARGS"));
    let unknown = guard.call(
        "desktop_run_steps",
        steps(json!([{"action": "click", "ref_id": "@sother:e1"}])),
    );
    assert_eq!(error_code(&unknown).as_deref(), Some("UNKNOWN_REF"));
    let odd = guard.call(
        "desktop_run_steps",
        steps(json!([{"action": "launch", "name": "x"}])),
    );
    assert_eq!(error_code(&odd).as_deref(), Some("INVALID_ARGS"));
    // A window that isn't the task's is refused before any step.
    let theirs = guard.call(
        "desktop_run_steps",
        args(json!({"window_id": CITRIX, "steps": [{"action": "press", "combo": "return"}]})),
    );
    assert_eq!(error_code(&theirs).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_run_steps"]);
}

#[test]
fn a_consequential_step_needs_the_persons_yes_and_its_result_is_checked() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    let batch = |reference: Value| {
        args(json!({"window_id": SETTINGS, "steps": [
            {"action": "type", "ref_id": "@sform:e3", "text": "Ada"},
            reference,
        ]}))
    };
    // By its ref, or by the name a step finds it by.
    for send in [
        json!({"action": "click", "ref_id": "@sform:e1"}),
        json!({"action": "click", "role": "button", "name": "Send"}),
    ] {
        let refused = guard.call("desktop_run_steps", batch(send));
        assert_eq!(error_code(&refused).as_deref(), Some("NEEDS_CONFIRMATION"));
        assert!(
            message(&refused).contains("step 2"),
            "{}",
            message(&refused)
        );
    }
    assert_eq!(fake.forwarded(), ["desktop_snapshot"]);
    authorize(&scratch, &[Category::Send]);
    let sent = guard.call(
        "desktop_run_steps",
        batch(json!({"action": "click", "ref_id": "@sform:e1"})),
    );
    assert_eq!(error_code(&sent), None);
    assert!(
        notes(&sent)
            .iter()
            .any(|note| note.contains("desktop_verify")),
        "{:?}",
        notes(&sent)
    );
    // An ordinary step has no such note.
    let plain = guard.call(
        "desktop_run_steps",
        batch(json!({"action": "click", "ref_id": "@sform:e2"})),
    );
    assert!(
        !notes(&plain)
            .iter()
            .any(|note| note.contains("desktop_verify"))
    );
}

#[test]
fn a_menu_item_is_consequential_by_its_title() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    let trash = guard.call(
        "desktop_menu",
        args(json!({"window_id": SETTINGS, "path": ["File", "Move to Trash"]})),
    );
    assert_eq!(error_code(&trash).as_deref(), Some("NEEDS_CONFIRMATION"));
    let font = guard.call(
        "desktop_menu",
        args(json!({"window_id": SETTINGS, "path": ["Format", "Schrift", "Fett"]})),
    );
    assert_eq!(error_code(&font), None);
    let theirs = guard.call(
        "desktop_menu",
        args(json!({"window_id": FRONT, "path": ["Format", "Schrift"]})),
    );
    assert_eq!(error_code(&theirs).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_menu"]);
}

#[test]
fn a_scripts_every_call_passes_the_guard() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    let run = |guard: &mut Guard<Fake>, script: &str| {
        guard.call(
            "desktop_run_script",
            args(json!({"script": script, "window_id": SETTINGS, "timeout_ms": 5000})),
        )
    };
    let done = run(
        &mut guard,
        r#"
        const read = desk.snapshot();
        desk.click("@sform:e2");
        console.log("clicked", read.snapshot_id);
        let refused = null;
        try { desk.click("@sform:e1"); } catch (error) { refused = error.code; }
        let opened = null;
        try { desk.call("hex_open_app", {app: "Mail"}); } catch (error) { opened = error.code; }
        return {snapshot: read.snapshot_id, refused, opened};
        "#,
    );
    assert_eq!(error_code(&done), None, "{done}");
    let data = child::data(&done).unwrap();
    assert_eq!(data["value"]["snapshot"], "sform");
    assert_eq!(data["value"]["refused"], "NEEDS_CONFIRMATION");
    assert_eq!(data["value"]["opened"], "POLICY_DENIED");
    assert_eq!(data["console"][0], "clicked sform");
    assert_eq!(data["calls_made"], 4);
    assert_eq!(
        fake.forwarded(),
        ["desktop_snapshot", "desktop_snapshot", "desktop_click"]
    );
    // A refusal it doesn't catch ends it, with its code.
    let theirs = run(
        &mut guard,
        r#"desk.call("desktop_snapshot", {window_id: "w-14569"}); return 1;"#,
    );
    assert_eq!(error_code(&theirs).as_deref(), Some("SCRIPT_FAILED"));
    assert!(
        message(&theirs).contains("NOT_YOUR_WINDOW"),
        "{}",
        message(&theirs)
    );
    // No way out of the sandbox, and no endless loop.
    let sandbox = run(
        &mut guard,
        r#"return [typeof require, typeof process, typeof fetch, typeof setTimeout];"#,
    );
    assert_eq!(
        child::data(&sandbox).unwrap()["value"],
        json!(["undefined", "undefined", "undefined", "undefined"])
    );
    let endless = guard.call(
        "desktop_run_script",
        args(json!({"script": "while (true) {}", "timeout_ms": 300})),
    );
    assert_eq!(error_code(&endless).as_deref(), Some("SCRIPT_FAILED"));
    assert!(
        message(&endless).contains("timed out"),
        "{}",
        message(&endless)
    );
    // A check runs no script at all.
    let mut check = task(&scratch, &fake, Autonomy::Smart);
    check.config.observe_only = true;
    let refused = run(&mut check, "return 1;");
    assert_eq!(error_code(&refused).as_deref(), Some("POLICY_DENIED"));
}

#[test]
fn the_whole_screen_is_for_full_control_and_the_clipboard_asks_a_job_first() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = task(&scratch, &fake, Autonomy::Smart);
    assert_eq!(
        error_code(&guard.call("desktop_screen", Map::new())).as_deref(),
        Some("POLICY_DENIED")
    );
    let offered = |guard: &mut Guard<Fake>| {
        guard
            .tools()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    assert!(!offered(&mut guard).contains(&"desktop_screen".to_owned()));
    for name in [
        "desktop_run_steps",
        "desktop_run_script",
        "desktop_menu",
        "desktop_verify",
        "desktop_drag",
        "desktop_zoom",
        "hex_skills",
        "hex_skill_record",
        "hex_skill_replay",
    ] {
        assert!(offered(&mut guard).contains(&name.to_owned()), "{name}");
    }
    guard.config.front_window = true;
    assert!(offered(&mut guard).contains(&"desktop_screen".to_owned()));
    assert_eq!(error_code(&guard.call("desktop_screen", Map::new())), None);
    // A job with real input may read the clipboard; replacing it asks.
    let mut physical = task(&scratch, &fake, Autonomy::Smart);
    physical.config.physical_input = true;
    assert_eq!(
        error_code(&physical.call("desktop_clipboard_get", Map::new())),
        None
    );
    let set = physical.call("desktop_clipboard_set", args(json!({"text": "x"})));
    assert_eq!(error_code(&set).as_deref(), Some("NEEDS_CONFIRMATION"));
    authorize(&scratch, &[Category::Share]);
    assert_eq!(
        error_code(&physical.call("desktop_clipboard_set", args(json!({"text": "x"})))),
        None
    );
}

#[test]
fn a_procedure_that_worked_is_kept_and_done_again_with_its_check() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    let record =
        |guard: &mut Guard<Fake>, value: Value| guard.call("hex_skill_record", args(value));
    let started = record(
        &mut guard,
        json!({"action": "start", "app": "Systemeinstellungen", "intent": "Set the name"}),
    );
    assert_eq!(error_code(&started), None, "{started}");
    guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e3", "text": "Ada"})),
    );
    guard.call("desktop_click", args(json!({"ref_id": "@sform:e2"})));
    guard.call(
        "desktop_type",
        args(json!({"ref_id": "@sform:e4", "text": " Thanks"})),
    );
    guard.call(
        "desktop_menu",
        args(json!({"window_id": SETTINGS, "path": ["Ablage", "Sichern"]})),
    );
    let stopped = record(
        &mut guard,
        json!({"action": "stop", "succeeded": true,
               "expect": [{"role": "textfield", "name": "Name", "value": "Ada"}]}),
    );
    assert_eq!(child::data(&stopped).unwrap()["saved"], true, "{stopped}");
    // Cua Driver recorded the evidence beside it.
    assert!(
        fake.forwarded()
            .contains(&"desktop_record_start".to_owned())
    );
    assert!(fake.forwarded().contains(&"desktop_record_stop".to_owned()));
    let kept: Value = serde_json::from_str(
        &std::fs::read_to_string(
            scratch
                .0
                .join("skills/systemeinstellungen/set-the-name/skill.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        kept["steps"],
        json!([
            {"action": "type", "role": "textfield", "name": "Name", "text": "Ada"},
            {"action": "click", "role": "button", "name": "OK"},
            // Found by its role next time: its text will differ.
            {"action": "type", "role": "textarea", "text": " Thanks"},
            {"action": "menu", "path": ["Ablage", "Sichern"]},
        ])
    );
    let listed = guard.call("hex_skills", Map::new());
    assert_eq!(
        child::data(&listed).unwrap()["skills"][0]["intent"],
        "Set the name"
    );
    // Done again in one call: a batch by names, the menu, then the check.
    fake.state().calls.clear();
    fake.state().replies.insert(
        "desktop_verify".into(),
        ok("verify", json!({"status": "satisfied"})),
    );
    let replayed = guard.call(
        "hex_skill_replay",
        args(json!({"intent": "set the name", "window_id": SETTINGS, "values": ["Grace"]})),
    );
    assert_eq!(error_code(&replayed), None, "{replayed}");
    assert_eq!(child::data(&replayed).unwrap()["worked"], true);
    assert_eq!(
        fake.forwarded(),
        ["desktop_run_steps", "desktop_menu", "desktop_verify"]
    );
    let batch = fake.last_args("desktop_run_steps");
    assert_eq!(batch["steps"][0]["text"], "Grace");
    assert_eq!(batch["steps"][1]["name"], "OK");
    let kept: Value = serde_json::from_str(
        &std::fs::read_to_string(
            scratch
                .0
                .join("skills/systemeinstellungen/set-the-name/skill.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (kept["runs"].clone(), kept["successes"].clone()),
        (json!(1), json!(1))
    );
    // Its steps pass the guard: in a window not the task's, nothing runs.
    fake.state().calls.clear();
    let theirs = guard.call(
        "hex_skill_replay",
        args(json!({"intent": "Set the name", "window_id": CITRIX})),
    );
    assert_eq!(error_code(&theirs).as_deref(), Some("SKILL_FAILED"));
    assert!(fake.forwarded().is_empty());
    // What didn't work isn't kept.
    record(
        &mut guard,
        json!({"action": "start", "app": "Systemeinstellungen", "intent": "Other"}),
    );
    let dropped = record(&mut guard, json!({"action": "stop", "succeeded": false}));
    assert_eq!(child::data(&dropped).unwrap()["saved"], false);
    assert!(
        !scratch
            .0
            .join("skills/systemeinstellungen/other/skill.json")
            .exists()
    );
}

#[test]
fn the_spot_a_ref_action_landed_at_rings_in_the_live_picture() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = holding_settings(&scratch, &fake, Autonomy::Smart);
    guard.call("desktop_click", args(json!({"ref_id": "@sform:e2"})));
    let (number, (x, y)) = guard.spotted.expect("a spot");
    assert_eq!(number, 18628);
    // OK's center (720, 960) in Settings (337, 71, 825 × 949).
    assert!((x - (720.0 - 337.0) / 825.0).abs() < 1e-9);
    assert!((y - (960.0 - 71.0) / 949.0).abs() < 1e-9);
    // A batch's spot is its last ref's.
    guard.spotted = None;
    guard.call(
        "desktop_run_steps",
        args(json!({"window_id": SETTINGS, "steps": [{"action": "click", "ref_id": "@sform:e2"}]})),
    );
    assert_eq!(guard.spotted.map(|(number, _)| number), Some(18628));
}
