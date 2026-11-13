//! Reads beside the guard (#36): a snapshot, the installed apps, or the
//! displays don't wait behind an action that holds the guard, are checked
//! against the task's leases as they are, and teach the guard what they
//! saw.

use std::time::Instant;

use super::*;
use crate::assistant::autonomy::Autonomy;

fn snapshot(window: &str) -> Map<String, Value> {
    args(json!({"window_id": window}))
}

#[test]
fn only_reads_run_beside_the_guard() {
    let none = Map::new();
    for tool in [
        "desktop_snapshot",
        "desktop_find",
        "desktop_get",
        "desktop_screenshot",
        policy::INSTALLED_APPS_TOOL,
        policy::DISPLAYS_TOOL,
        policy::APP_CAPABILITIES_TOOL,
        policy::WINDOW_SHOT_TOOL,
        policy::READ_WINDOW_TOOL,
    ] {
        assert!(policy::kind(tool, &none).reads_alone(), "{tool}");
    }
    for tool in [
        "desktop_click",
        "desktop_press",
        "desktop_type",
        "desktop_launch",
        "desktop_list_windows",
        policy::OPEN_APP_TOOL,
        policy::CLAIM_TOOL,
        policy::WINDOW_INPUT_TOOL,
        policy::APP_ACTION_TOOL,
    ] {
        assert!(!policy::kind(tool, &none).reads_alone(), "{tool}");
    }
    // A reader refuses anything else, should one reach it.
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let mut reader = guard.seed().reader();
    let click = reader.call("desktop_click", args(json!({"ref_id": "@s1:e1"})));
    assert_eq!(click["isError"], true);
    assert!(fake.forwarded().is_empty());
}

#[test]
fn a_read_runs_while_an_action_waits_for_a_paused_task() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let read = guard.call("desktop_snapshot", snapshot(SETTINGS));
    let reference = child::data(&read).unwrap()["tree"]["children"][0]["ref_id"].clone();
    let seed = guard.seed();
    let guard = Arc::new(Mutex::new(guard));
    let store = scratch.store();
    store.set_paused("bg", true, 1);
    // The action holds the guard for as long as the task is paused.
    let acting = {
        let guard = guard.clone();
        std::thread::spawn(move || {
            guard
                .lock()
                .unwrap()
                .call("desktop_click", args(json!({ "ref_id": reference })))
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.try_lock().is_ok() {
        assert!(Instant::now() < deadline, "the action never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Reads go on beside it, each in a reader of its own, in parallel.
    let started = Instant::now();
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let mut reader = seed.reader();
            std::thread::spawn(move || reader.call("desktop_snapshot", snapshot(SETTINGS)))
        })
        .collect();
    for reader in readers {
        assert_eq!(error_code(&reader.join().unwrap()), None);
    }
    let mut reader = seed.reader();
    assert_eq!(
        error_code(&reader.call(policy::DISPLAYS_TOOL, Map::new())),
        None
    );
    assert_eq!(
        error_code(&reader.call(policy::INSTALLED_APPS_TOOL, Map::new())),
        None
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!acting.is_finished(), "the action still waits");
    store.set_paused("bg", false, 2);
    let acted = acting.join().unwrap();
    assert_eq!(error_code(&acted).as_deref(), Some("TAKEN_OVER"));
}

#[test]
fn a_reader_checks_the_leases_as_they_are_now() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    let seed = guard.seed();
    // Not the task's yet.
    let refused = seed.reader().call("desktop_snapshot", snapshot(SETTINGS));
    assert_eq!(error_code(&refused).as_deref(), Some("NOT_YOUR_WINDOW"));
    // The guard claims it after the seed was taken: the reader sees it.
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let read = seed.reader().call("desktop_snapshot", snapshot(SETTINGS));
    assert_eq!(error_code(&read), None);
    // Another task's window stays refused.
    let other = Other::new();
    let now = crate::desktop::events::now_ms();
    assert_eq!(
        scratch
            .store()
            .acquire("other", other.pid(), CITRIX, "Citrix Viewer", now),
        Acquire::Acquired
    );
    let theirs = seed.reader().call("desktop_snapshot", snapshot(CITRIX));
    assert_eq!(error_code(&theirs).as_deref(), Some("NOT_YOUR_WINDOW"));
    // Released: the reader stops reading it too.
    scratch.store().release("bg", SETTINGS);
    let released = seed.reader().call("desktop_snapshot", snapshot(SETTINGS));
    assert_eq!(error_code(&released).as_deref(), Some("NOT_YOUR_WINDOW"));
}

#[test]
fn what_a_reader_saw_the_guard_knows() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.config.autonomy = Autonomy::Smart;
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let seed = guard.seed();
    fake.state().replies.insert(
        "desktop_snapshot".into(),
        ok(
            "snapshot",
            json!({
                "app": "System Settings",
                "snapshot_id": "sread",
                "window": {"id": SETTINGS, "title": ""},
                "tree": {"role": "window", "children": [
                    {"role": "button", "name": "Delete", "ref_id": "@sread:e1"},
                    {"role": "textfield", "name": "Password", "states": ["secure"],
                     "value": "hunter2", "ref_id": "@sread:e2"},
                ]},
            }),
        ),
    );
    let read = seed.reader().call("desktop_snapshot", snapshot(SETTINGS));
    assert!(!read.to_string().contains("hunter2"));
    // The guard knows the reader's refs: their window, label, and secrets.
    let click = guard.call("desktop_click", args(json!({"ref_id": "@sread:e1"})));
    assert_eq!(error_code(&click).as_deref(), Some("NEEDS_CONFIRMATION"));
    let secret = guard.call(
        "desktop_get",
        args(json!({"ref_id": "@sread:e2", "property": "value"})),
    );
    assert_eq!(error_code(&secret).as_deref(), Some("POLICY_DENIED"));
}
