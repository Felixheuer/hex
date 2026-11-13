//! The guard's tests, round two: one look at the screen for close checks,
//! a Here task's dialogs, reused pids, the clipboard, and Hex's app tools.

use super::*;

#[test]
fn checks_close_together_look_at_the_screen_once() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.fresh_for = Duration::from_secs(60);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    for _ in 0..3 {
        guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
        guard.call("desktop_snapshot", args(json!({"window_id": FRONT})));
    }
    assert_eq!(
        fake.state().looks,
        1,
        "one look stands for the checks after it"
    );
    let reference =
        child::data(&guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})))).unwrap()
            ["tree"]["children"][0]["ref_id"]
            .clone();
    guard.call("desktop_click", args(json!({"ref_id": reference})));
    assert_eq!(
        fake.state().looks,
        2,
        "after its own action the guard looks again"
    );
    // The window list never goes through agent-desktop.
    assert!(
        fake.state()
            .calls
            .iter()
            .all(|(tool, _)| tool != "desktop_list_windows")
    );
}

#[test]
fn core_graphics_windows_are_named_and_the_focused_apps_first_is_in_front() {
    let raw = |id: u32, pid: i32, owner: &str, layer: i32| scrawl_capture::windows::RawWindow {
        id,
        pid,
        owner: owner.into(),
        title: String::new(),
        bounds: [0.0, 0.0, 400.0, 300.0],
        layer,
        alpha: 1.0,
    };
    let windows = super::super::screen::windows_among(
        &[
            raw(1, 9, "Dock", 20),
            raw(40, 7, "Notes", 0),
            raw(41, 8, "Mail", 0),
            raw(42, 8, "Mail", 0),
        ],
        Some(8),
        None,
    );
    let front = |windows: &[child::Window]| -> Vec<String> {
        windows
            .iter()
            .filter(|window| window.focused)
            .map(|window| window.id.clone())
            .collect()
    };
    assert_eq!(front(&windows), ["w-41"]);
    assert!(!windows[0].visible, "the Dock isn't an ordinary window");
    assert_eq!(windows[1].app, "Notes");
    assert_eq!(windows[1].pid, Some(7));
    // The window the app says has focus wins over its frontmost one (a
    // browser's strip or panel can be in front of its window).
    let said = super::super::screen::windows_among(
        &[raw(41, 8, "Mail", 0), raw(42, 8, "Mail", 0)],
        Some(8),
        Some(42),
    );
    assert_eq!(front(&said), ["w-42"]);
    // Without Accessibility to ask, the first ordinary window.
    let guessed = super::super::screen::windows_among(&[raw(40, 7, "Notes", 0)], None, None);
    assert_eq!(front(&guessed), ["w-40"]);
}

#[test]
fn a_here_task_holds_the_dialog_its_action_opens_but_nothing_of_other_apps() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    // Its click opens a dialog in its app (in front) and, at the same
    // moment, another app opens a window.
    struct Opening(Fake);
    impl Desktop for Opening {
        fn tools(&mut self) -> Result<Vec<Value>, String> {
            self.0.tools()
        }
        fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
            if tool == "desktop_click" {
                let mut state = self.0.state();
                for window in &mut state.windows {
                    window["is_focused"] = json!(false);
                }
                state.windows.insert(
                    0,
                    json!({
                        "app_name": "Hex Test", "id": "w-700", "is_focused": true, "pid": 81316,
                        "title": "Save", "visible": true,
                    }),
                );
                state.windows.push(json!({
                    "app_name": "Notes", "id": "w-701", "is_focused": false, "pid": 6000,
                    "title": "Elsewhere", "visible": true,
                }));
            }
            self.0.call(tool, args)
        }
    }
    let mut guard_with = Guard::new(
        Config {
            task: "here-task".into(),
            lane: Lane::Here,
            here_window: Some(TaskWindow {
                app: "Hex Test".into(),
                window_id: Some(FRONT.into()),
            }),
            physical_input: false,
            blocked_apps: Vec::new(),
            agent_display: None,
            autonomy: Default::default(),
            front_window: false,
            open_apps: false,
            observe_only: false,
        },
        scratch.store(),
        std::process::id(),
        Opening(fake.clone()),
        Box::new(fake.clone()),
    );
    guard_with.fresh_for = Duration::ZERO;
    let snapshot = guard_with.call("desktop_snapshot", args(json!({"window_id": FRONT})));
    let reference = child::data(&snapshot).unwrap()["tree"]["children"][0]["ref_id"].clone();
    let clicked = guard_with.call("desktop_click", args(json!({"ref_id": reference})));
    assert!(
        notes(&clicked).iter().any(|note| note.contains("w-700")),
        "{clicked}"
    );
    assert_eq!(scratch.store().holder("w-700").unwrap().task, "here-task");
    assert!(scratch.store().holder("w-701").is_none());
    let dialog = guard_with.call("desktop_snapshot", args(json!({"window_id": "w-700"})));
    assert_eq!(error_code(&dialog), None);
    // It may look at another window, but not act in it.
    let looked = guard_with.call("desktop_snapshot", args(json!({"window_id": "w-701"})));
    assert_eq!(error_code(&looked), None);
    let other = guard_with.call("desktop_minimize", args(json!({"window_id": "w-701"})));
    assert_eq!(error_code(&other).as_deref(), Some("NOT_YOUR_WINDOW"));
    // A window of its app the person opens later isn't the task's.
    fake.state().windows.push(json!({
        "app_name": "Hex Test", "id": "w-702", "is_focused": false, "pid": 81316,
        "title": "Theirs", "visible": true,
    }));
    guard_with.call("desktop_snapshot", args(json!({"window_id": FRONT})));
    assert!(scratch.store().holder("w-702").is_none());
}

#[test]
fn a_reused_pid_does_not_keep_a_lease() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let me = std::process::id();
    assert!(super::super::state::started_us(me).is_some());
    assert_eq!(store.acquire("a", me, "w-1", "Mail", 1), Acquire::Acquired);
    assert_eq!(
        store.holder("w-1").unwrap().started,
        super::super::state::started_us(me)
    );
    // The same pid, but a process that started at another time: the
    // system gave the dead holder's pid to someone else.
    let path = scratch.0.join("leases/w-2.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"task": "old", "pid": me, "window_id": "w-2", "app": "Mail", "since_ms": 1,
            "started": 1})
        .to_string(),
    )
    .unwrap();
    assert!(store.holder("w-2").is_none());
    assert_eq!(
        store.acquire("new", me, "w-2", "Mail", 2),
        Acquire::Acquired
    );
    // A lease from before start times were kept still counts while its pid runs.
    std::fs::write(
        scratch.0.join("leases/w-3.json"),
        json!({"task": "older", "pid": me, "window_id": "w-3", "app": "Mail", "since_ms": 1})
            .to_string(),
    )
    .unwrap();
    assert_eq!(store.holder("w-3").unwrap().task, "older");
}

#[test]
fn the_clipboard_is_for_here_tasks_or_with_real_input() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut background_task = background(&scratch, &fake, "bg", false);
    let refused = background_task.call("desktop_clipboard_set", args(json!({"text": "x"})));
    assert_eq!(error_code(&refused).as_deref(), Some("POLICY_DENIED"));
    assert!(message(&refused).contains("desktop_type"));
    let tools = background_task.tools().unwrap();
    assert!(
        tools
            .iter()
            .all(|tool| tool["name"] != "desktop_clipboard_get")
    );
    assert!(fake.forwarded().is_empty());
    let mut physical = background(&scratch, &fake, "bg2", true);
    assert_eq!(
        error_code(&physical.call("desktop_clipboard_get", Map::new())),
        None
    );
    let mut here_task = here(&scratch, &fake, FRONT, "Hex Test", false);
    assert_eq!(
        error_code(&here_task.call("desktop_clipboard_set", args(json!({"text": "x"})))),
        None
    );
    assert_eq!(
        fake.forwarded(),
        ["desktop_clipboard_get", "desktop_clipboard_set"]
    );
}

fn textedit() -> App {
    App {
        name: "TextEdit".into(),
        bundle_id: Some("com.apple.TextEdit".into()),
        path: "/System/Applications/TextEdit.app".into(),
    }
}

fn with_apps(fake: &Fake) {
    let mut state = fake.state();
    state.apps = vec![
        textedit(),
        App {
            name: "Ghostty".into(),
            bundle_id: Some("com.mitchellh.ghostty".into()),
            path: "/Applications/Ghostty.app".into(),
        },
        App {
            name: "Notes".into(),
            bundle_id: Some("com.apple.Notes".into()),
            path: "/System/Applications/Notes.app".into(),
        },
    ];
    state.running = vec!["/Applications/Ghostty.app".into()];
}

#[test]
fn installed_apps_are_listed_with_what_runs_and_what_is_blocked() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_apps(&fake);
    let mut guard = background(&scratch, &fake, "bg", false);
    let names: Vec<Value> = guard
        .tools()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].clone())
        .collect();
    assert!(names.contains(&json!("hex_installed_apps")));
    assert!(names.contains(&json!("hex_open_app")));
    let listed = guard.call("hex_installed_apps", Map::new());
    let apps = child::data(&listed).unwrap()["apps"].clone();
    assert_eq!(apps.as_array().unwrap().len(), 3);
    assert_eq!(apps[0]["name"], "Ghostty", "sorted by name");
    assert_eq!(apps[0]["running"], true);
    assert_eq!(apps[0]["blocked"], true);
    assert_eq!(apps[2]["bundle_id"], "com.apple.TextEdit");
    assert_eq!(apps[2]["running"], false);
    let found = guard.call("hex_installed_apps", args(json!({"query": "apple.note"})));
    let apps = child::data(&found).unwrap()["apps"].clone();
    assert_eq!(apps.as_array().unwrap().len(), 1);
    assert_eq!(apps[0]["name"], "Notes");
    assert!(
        fake.forwarded().is_empty(),
        "read in Hex, not through agent-desktop"
    );
    // A Here task may look, but not open.
    let mut here_task = here(&scratch, &fake, FRONT, "Hex Test", false);
    let tools = here_task.tools().unwrap();
    assert!(tools.iter().all(|tool| tool["name"] != "hex_open_app"));
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "hex_installed_apps")
    );
    let refused = here_task.call("hex_open_app", args(json!({"name": "TextEdit"})));
    assert_eq!(error_code(&refused).as_deref(), Some("POLICY_DENIED"));
}

#[test]
fn installed_apps_say_which_kinds_of_links_each_opens() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_apps(&fake);
    fake.state().link_handlers = Some(json!({"LSHandlers": [
        {"LSHandlerURLScheme": "mailto", "LSHandlerRoleAll": "com.apple.notes"},
        {"LSHandlerURLScheme": "https", "LSHandlerRoleAll": "com.mitchellh.ghostty"},
    ]}));
    let mut guard = background(&scratch, &fake, "bg", false);
    let listed = guard.call("hex_installed_apps", Map::new());
    let apps = child::data(&listed).unwrap()["apps"].clone();
    let app = |name: &str| {
        apps.as_array()
            .unwrap()
            .iter()
            .find(|app| app["name"] == name)
            .cloned()
            .unwrap()
    };
    assert_eq!(app("Ghostty")["default_for"], json!(["https"]));
    assert_eq!(app("Notes")["default_for"], json!(["mailto"]));
    assert!(app("TextEdit").get("default_for").is_none());
}

#[test]
fn opening_an_app_waits_for_its_new_window_and_leases_it() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_apps(&fake);
    fake.state().on_launch = Some(json!({
        "app_name": "TextEdit", "id": "w-800", "is_focused": false, "pid": 900,
        "title": "Untitled", "visible": true,
    }));
    let mut guard = background(&scratch, &fake, "bg", false);
    let opened = guard.call("hex_open_app", args(json!({"name": "textedit"})));
    let data = child::data(&opened).expect("opened");
    assert_eq!(data["windows"][0]["id"], "w-800");
    assert_eq!(data["was_running"], false);
    assert_eq!(scratch.store().holder("w-800").unwrap().task, "bg");
    // Launched by bundle id, never brought in front.
    let launch = fake.last_args("desktop_launch");
    assert_eq!(launch["app"], "com.apple.TextEdit");
    assert!(launch.get("activate").is_none());
    // A fresh instance when asked (no window comes: it says so after waiting).
    let notes_app = guard.call(
        "hex_open_app",
        args(json!({"bundle_id": "com.apple.Notes", "new_instance": true})),
    );
    assert_eq!(fake.last_args("desktop_launch")["no_attach"], true);
    assert!(
        child::data(&notes_app).unwrap()["note"]
            .as_str()
            .unwrap()
            .contains("no window yet")
    );
    for (call, code) in [
        (json!({"name": "Ghostty"}), "BLOCKED_APP"),
        (json!({"name": "Nonexistent"}), "APP_NOT_FOUND"),
        (json!({}), "INVALID_ARGS"),
    ] {
        let refused = guard.call("hex_open_app", args(call));
        assert_eq!(error_code(&refused).as_deref(), Some(code));
    }
    assert_eq!(fake.forwarded(), ["desktop_launch", "desktop_launch"]);
}

#[test]
fn a_held_window_on_another_space_is_still_the_tasks() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    // It leaves the screen (moved to another Space): CoreGraphics no longer
    // lists it, but it is still the task's.
    fake.state()
        .windows
        .retain(|window| window["id"] != SETTINGS);
    let read = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(error_code(&read), None, "{read}");
    let by_app = guard.call(
        "desktop_snapshot",
        args(json!({"app": "System\u{ad}einstellungen"})),
    );
    assert_eq!(error_code(&by_app), None, "{by_app}");
    assert_eq!(fake.last_args("desktop_snapshot")["window_id"], SETTINGS);
}

#[test]
fn a_running_apps_bundle_is_found_from_its_executable() {
    use super::super::screen::bundle_of;
    assert_eq!(
        bundle_of(Path::new(
            "/System/Applications/Mail.app/Contents/MacOS/Mail"
        )),
        Some(PathBuf::from("/System/Applications/Mail.app"))
    );
    assert_eq!(bundle_of(Path::new("/usr/bin/true")), None);
}

/// Live, read-only: the installed apps and the windows on screen, read in
/// Hex, fast. Skipped where Hex can see no window (a headless runner).
#[cfg(target_os = "macos")]
#[test]
fn this_macs_apps_and_windows_are_read_in_milliseconds() {
    use super::super::screen::CoreGraphics;
    let mut screen = CoreGraphics;
    let started = std::time::Instant::now();
    let _ = screen.installed_apps();
    let first_apps = started.elapsed();
    let started = std::time::Instant::now();
    let _ = screen.windows();
    let first_windows = started.elapsed();
    let started = std::time::Instant::now();
    let apps = screen.installed_apps();
    let running = screen.running_apps();
    let apps_took = started.elapsed();
    let started = std::time::Instant::now();
    let windows = screen.windows();
    let windows_took = started.elapsed();
    let started = std::time::Instant::now();
    let raw = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
    let raw_took = started.elapsed();
    let started = std::time::Instant::now();
    let focused = scrawl_capture::ax::focused_app_pid();
    let focused_took = started.elapsed();
    eprintln!(
        "first: apps {first_apps:?}, windows {first_windows:?}; then {} apps, {} running, in \
         {apps_took:?}; {} windows in {windows_took:?} (CoreGraphics {raw_took:?} for {}, \
         focused app {focused:?} {focused_took:?})",
        apps.len(),
        running.len(),
        windows.len(),
        raw.len()
    );
    if windows.is_empty() {
        return;
    }
    assert!(apps_took < Duration::from_millis(50), "{apps_took:?}");
    assert!(windows_took < Duration::from_millis(50), "{windows_took:?}");
    assert!(windows.iter().all(|window| window.id.starts_with("w-")));
}
