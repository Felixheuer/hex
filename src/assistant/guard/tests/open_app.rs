//! `hex_open_app` for an app that already runs (never a second instance),
//! and full control in the Here lane ("open WhatsApp" from a hold).

use super::*;

fn app(name: &str, bundle_id: &str) -> App {
    App {
        name: name.into(),
        bundle_id: Some(bundle_id.into()),
        path: format!("/Applications/{name}.app").into(),
    }
}

/// Citrix Viewer (on screen, not in front), WhatsApp, and Notes run;
/// TextEdit is installed only.
fn with_running_apps(fake: &Fake) {
    let mut state = fake.state();
    state.apps = vec![
        app("Citrix Viewer", "com.citrix.receiver.icaviewer.mac"),
        app("WhatsApp", "net.whatsapp.WhatsApp"),
        app("Notes", "com.apple.Notes"),
        App {
            name: "TextEdit".into(),
            bundle_id: Some("com.apple.TextEdit".into()),
            path: "/System/Applications/TextEdit.app".into(),
        },
    ];
    state.running = vec![
        "/Applications/Citrix Viewer.app".into(),
        "/Applications/WhatsApp.app".into(),
        "/Applications/Notes.app".into(),
    ];
}

fn data(result: &Value) -> Value {
    child::data(result).unwrap_or_else(|| panic!("no data: {result}"))
}

#[test]
fn a_running_apps_window_on_screen_is_taken_without_starting_it_again() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = background(&scratch, &fake, "bg", false);
    let opened = guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    let data = data(&opened);
    assert_eq!(data["was_running"], true);
    assert_eq!(data["windows"][0]["id"], CITRIX);
    assert_eq!(data["windows"][0]["on_screen"], true);
    assert_eq!(scratch.store().holder(CITRIX).unwrap().task, "bg");
    // Nothing launched, nothing reopened: no second instance, no waiting.
    assert!(fake.forwarded().is_empty(), "{:?}", fake.forwarded());
    // It is the task's now: acting in it passes.
    let clicked = guard.call(
        "desktop_snapshot",
        args(json!({"window_id": CITRIX, "interactive_only": true})),
    );
    assert_eq!(error_code(&clicked), None, "{clicked}");
}

#[test]
fn a_running_apps_window_on_another_desktop_is_taken_as_it_is() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    fake.state().off_screen = vec![json!({
        "id": "w-900", "app_name": "WhatsApp", "title": "WhatsApp", "pid": 4100,
        "is_focused": false, "visible": false,
    })];
    let reopened = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut guard = background(&scratch, &fake, "bg", false);
    let seen = reopened.clone();
    guard.reopen = Box::new(move |app| seen.lock().unwrap().push(app.name.clone()));
    let opened = guard.call("hex_open_app", args(json!({"name": "whatsapp"})));
    let data = data(&opened);
    assert_eq!(data["windows"][0]["id"], "w-900");
    assert_eq!(data["windows"][0]["on_screen"], false);
    assert!(data["note"].as_str().unwrap().contains("another desktop"));
    assert!(
        reopened.lock().unwrap().is_empty(),
        "it has a window: no reopen"
    );
    assert!(fake.forwarded().is_empty());
    // Off the screen, it is still the task's to read and act in.
    let read = guard.call("desktop_snapshot", args(json!({"window_id": "w-900"})));
    assert_eq!(error_code(&read), None, "{read}");
}

#[test]
fn a_running_app_without_a_window_is_asked_to_reopen_one() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let reopened = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut guard = background(&scratch, &fake, "bg", false);
    let (seen, screen) = (reopened.clone(), fake.clone());
    guard.reopen = Box::new(move |app| {
        seen.lock().unwrap().push(app.name.clone());
        // Notes shows its window again.
        screen.state().windows.push(json!({
            "app_name": "Notes", "id": "w-950", "is_focused": false, "pid": 4200,
            "title": "Notes", "visible": true,
        }));
    });
    let opened = guard.call("hex_open_app", args(json!({"name": "Notes"})));
    let data = data(&opened);
    assert_eq!(*reopened.lock().unwrap(), ["Notes"]);
    assert_eq!(data["windows"][0]["id"], "w-950");
    assert_eq!(data["was_running"], true);
    assert_eq!(scratch.store().holder("w-950").unwrap().task, "bg");
    assert!(
        !fake.forwarded().contains(&"desktop_launch".to_owned()),
        "a running app is never launched again"
    );
}

#[test]
fn another_tasks_window_of_a_running_app_is_left_to_it() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut other = background(&scratch, &fake, "other", false);
    other.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    let mut guard = background(&scratch, &fake, "bg", false);
    let opened = guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    let data = data(&opened);
    assert_eq!(data["windows"], json!([]));
    assert!(data["note"].as_str().unwrap().contains("another task"));
    assert_eq!(scratch.store().holder(CITRIX).unwrap().task, "other");
    assert!(fake.forwarded().is_empty());
}

#[test]
fn with_full_control_a_here_task_opens_and_switches_to_apps() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    guard.config.front_window = true;
    let names: Vec<Value> = guard
        .tools()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].clone())
        .collect();
    for wanted in ["hex_open_app", "desktop_claim_window", "hex_installed_apps"] {
        assert!(names.contains(&json!(wanted)), "{wanted}");
    }
    assert!(guard.rules().contains("full control"));
    // A running app: its window is the task's at once.
    let switched = guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    assert_eq!(data(&switched)["windows"][0]["id"], CITRIX);
    assert_eq!(scratch.store().holder(CITRIX).unwrap().task, "here-task");
    let typed = guard.call(
        "desktop_snapshot",
        args(json!({"window_id": CITRIX, "interactive_only": true})),
    );
    assert_eq!(error_code(&typed), None, "{typed}");
    // An app that isn't running starts, and its new window is the task's.
    fake.state().on_launch = Some(json!({
        "app_name": "TextEdit", "id": "w-800", "is_focused": false, "pid": 900,
        "title": "Untitled", "visible": true,
    }));
    let opened = guard.call("hex_open_app", args(json!({"name": "TextEdit"})));
    let data = data(&opened);
    assert_eq!(data["windows"][0]["id"], "w-800", "{data}");
    assert_eq!(scratch.store().holder("w-800").unwrap().task, "here-task");
    assert!(fake.last_args("desktop_launch").get("activate").is_none());
    // Without full control the Here lane still stays in its window.
    let mut kept = here(&scratch, &fake, FRONT, "Hex Test", false);
    kept.config.task = "kept".into();
    let refused = kept.call("hex_open_app", args(json!({"name": "Notes"})));
    assert_eq!(error_code(&refused).as_deref(), Some("POLICY_DENIED"));
}

#[test]
fn asked_to_stay_in_the_background_a_here_task_still_opens_apps_of_its_own() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    // "Kannst du jede beliebige App nur im Hintergrund bedienen?"
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    guard.config.open_apps = true;
    let tools = guard.tools().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "hex_open_app"));
    assert!(guard.rules().contains("stays out of the person's windows"));
    let opened = guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    assert_eq!(data(&opened)["windows"][0]["id"], CITRIX);
    let read = guard.call("desktop_snapshot", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&read), None, "{read}");
    // The person's other windows stay theirs.
    let refused = guard.call("desktop_minimize", args(json!({"window_id": SETTINGS})));
    assert_eq!(error_code(&refused).as_deref(), Some("NOT_YOUR_WINDOW"));
}
