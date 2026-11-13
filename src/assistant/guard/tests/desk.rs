//! "Öffne WhatsApp" when its window is on another desktop or in full
//! screen (recovery), the desk that keeps a quick turn's window, and
//! background first (a brief focus only after background actions failed).

use super::*;

fn app(name: &str, bundle_id: &str) -> App {
    App {
        name: name.into(),
        bundle_id: Some(bundle_id.into()),
        path: format!("/Applications/{name}.app").into(),
    }
}

fn with_running_apps(fake: &Fake) {
    let mut state = fake.state();
    state.apps = vec![
        app("Citrix Viewer", "com.citrix.receiver.icaviewer.mac"),
        app("WhatsApp", "net.whatsapp.WhatsApp"),
        app("Notes", "com.apple.Notes"),
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

/// Recovery's steps, faked: a relaunch or leaving full screen brings
/// WhatsApp's window onto this screen; every step is kept.
struct FakeRecoverer {
    fake: Fake,
    steps: Arc<Mutex<Vec<String>>>,
    full_screen: bool,
}

impl super::super::proxy::Recoverer for FakeRecoverer {
    fn inspect(&mut self, _pid: i32) -> (bool, Vec<crate::assistant::actions::ax::AxWindow>) {
        let listed = self
            .full_screen
            .then_some(crate::assistant::actions::ax::AxWindow {
                number: 900,
                minimized: false,
                full_screen: true,
                edited: false,
            });
        (false, listed.into_iter().collect())
    }

    fn traits(&mut self, _app: &App) -> crate::assistant::actions::recovery::Traits {
        Default::default()
    }

    fn unhide(&mut self, _pid: i32) -> bool {
        self.steps.lock().unwrap().push("unhide".into());
        false
    }

    fn leave_full_screen(&mut self, _pid: i32, number: u32) -> bool {
        self.steps
            .lock()
            .unwrap()
            .push(format!("leave-full-screen {number}"));
        self.full_screen = false;
        self.arrive();
        true
    }

    fn new_window(&mut self, _script: &str) -> bool {
        self.steps.lock().unwrap().push("new-window".into());
        false
    }

    fn relaunch(&mut self, _pids: &[i64], app: &App) -> bool {
        self.steps
            .lock()
            .unwrap()
            .push(format!("relaunch {}", app.name));
        self.arrive();
        true
    }
}

impl FakeRecoverer {
    fn arrive(&self) {
        let mut state = self.fake.state();
        state.off_screen.clear();
        state.windows.push(json!({
            "app_name": "\u{200e}WhatsApp", "id": "w-901", "is_focused": false, "pid": 4101,
            "title": "WhatsApp", "visible": true,
        }));
    }
}

fn on_another_desktop(fake: &Fake) {
    fake.state().off_screen = vec![json!({
        "id": "w-900", "app_name": "\u{200e}WhatsApp", "title": "WhatsApp", "pid": 4100,
        "is_focused": false, "visible": false,
    })];
}

#[test]
fn whatsapp_on_another_desktop_is_reopened_here_and_stays_on_the_desk() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    on_another_desktop(&fake);
    let steps = Arc::new(Mutex::new(Vec::new()));
    // "Öffne WhatsApp" from a hold, with full control.
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    guard.config.front_window = true;
    guard.desk = Some(std::process::id());
    guard.recover = Box::new(FakeRecoverer {
        fake: fake.clone(),
        steps: steps.clone(),
        full_screen: false,
    });
    let opened = guard.call("hex_open_app", args(json!({"name": "WhatsApp"})));
    let found = data(&opened);
    // Not scriptable, keeps no documents: quit quietly and reopened.
    assert_eq!(*steps.lock().unwrap(), ["relaunch WhatsApp"]);
    assert_eq!(found["windows"][0]["id"], "w-901", "{found}");
    assert_eq!(found["windows"][0]["on_screen"], true);
    let note = found["note"].as_str().unwrap();
    assert!(note.contains("reopened it in the background"), "{note}");
    assert!(note.contains("desk"), "{note}");
    // The desk holds it, not the turn.
    assert_eq!(scratch.store().holder("w-901").unwrap().task, "desk");
    // The agent names the app without its invisible mark: it reaches
    // agent-desktop as the running app's own name.
    let read = guard.call(
        "desktop_snapshot",
        args(json!({"app": "WhatsApp", "window_id": "w-901"})),
    );
    assert_eq!(error_code(&read), None, "{read}");
    assert_eq!(
        fake.last_args("desktop_snapshot")["app"],
        "\u{200e}WhatsApp"
    );
    // The turn ends: its leases go, the desk's window stays.
    drop(guard);
    scratch.store().release_all("here-task");
    assert_eq!(scratch.store().holder("w-901").unwrap().task, "desk");
    // A later turn acts in it directly.
    let mut later = here(&scratch, &fake, FRONT, "Hex Test", false);
    later.config.task = "later".into();
    later.config.open_apps = true;
    let read = later.call("desktop_snapshot", args(json!({"window_id": "w-901"})));
    assert_eq!(error_code(&read), None, "{read}");
    // "Schließ WhatsApp": the desk lets go.
    let released = later.call("hex_release_window", args(json!({"app": "whatsapp"})));
    assert_eq!(data(&released)["released"][0]["id"], "w-901");
    assert!(scratch.store().holder("w-901").is_none());
}

#[test]
fn a_full_screen_window_leaves_full_screen_instead_of_a_relaunch() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    on_another_desktop(&fake);
    let steps = Arc::new(Mutex::new(Vec::new()));
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.recover = Box::new(FakeRecoverer {
        fake: fake.clone(),
        steps: steps.clone(),
        full_screen: true,
    });
    let opened = guard.call("hex_open_app", args(json!({"name": "WhatsApp"})));
    assert_eq!(*steps.lock().unwrap(), ["leave-full-screen 900"]);
    let found = data(&opened);
    assert_eq!(found["windows"][0]["id"], "w-901", "{found}");
    assert!(found["note"].as_str().unwrap().contains("full screen"));
    // A Background task's window is its own, never the desk's.
    assert_eq!(scratch.store().holder("w-901").unwrap().task, "bg");
}

#[test]
fn focusing_waits_for_background_actions_to_fail_first() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    guard.config.front_window = true;
    guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    // Background first: the first ask is refused with how to do without.
    let first = guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&first).as_deref(), Some("BACKGROUND_FIRST"));
    assert!(
        !fake
            .forwarded()
            .contains(&"desktop_focus_window".to_owned())
    );
    // Asked again, it goes ahead, briefly.
    let second = guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&second), None, "{second}");
    assert!(
        notes(&second)
            .iter()
            .any(|note| note.contains("in front briefly")),
        "{second}"
    );
    // Two failed actions in a window offer the brief focus.
    fake.state().replies.insert(
        "desktop_scroll".into(),
        json!({"content": [{"type": "text", "text": "{\"ok\":false}"}], "isError": true}),
    );
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": CITRIX})));
    let reference = child::data(&snapshot).unwrap()["tree"]["children"][0]["ref_id"]
        .as_str()
        .unwrap()
        .to_owned();
    guard.call(
        "desktop_scroll",
        args(json!({"ref_id": reference, "direction": "down"})),
    );
    let again = guard.call(
        "desktop_scroll",
        args(json!({"ref_id": reference, "direction": "down"})),
    );
    assert!(
        notes(&again)
            .iter()
            .any(|note| note.contains("ignored this in the background twice")),
        "{again}"
    );
}

#[test]
fn any_app_takes_a_url_and_recipes_check_blocked_apps() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut guard = background(&scratch, &fake, "bg", false);
    let seen = ran.clone();
    guard.run_recipe = Box::new(move |command| {
        seen.lock().unwrap().push(format!("{command:?}"));
        Ok(String::new())
    });
    let tools: Vec<String> = guard
        .tools()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect();
    assert!(tools.contains(&"hex_app_action".to_owned()));
    assert!(tools.contains(&"hex_read_window".to_owned()));
    // A draft, never sent.
    let drafted = guard.call(
        "hex_app_action",
        args(json!({"app": "whatsapp", "intent": "open_chat",
            "params": {"phone": "+49 176 1234567", "text": "Hallo"}})),
    );
    let note = data(&drafted)["note"].as_str().unwrap().to_owned();
    assert!(note.contains("nothing was sent"), "{note}");
    // Any app, no recipe needed.
    let opened = guard.call(
        "hex_app_action",
        args(json!({"app": "Notes", "intent": "open_url",
            "params": {"url": "notes://showNote?identifier=1"}})),
    );
    assert_eq!(error_code(&opened), None, "{opened}");
    assert_eq!(ran.lock().unwrap().len(), 2);
    // Ghostty is blocked for this task.
    let blocked = guard.call(
        "hex_app_action",
        args(json!({"app": "ghostty", "intent": "open", "params": {"dir": "/tmp"}})),
    );
    assert_eq!(error_code(&blocked).as_deref(), Some("BLOCKED_APP"));
    // Sending needs the person's yes.
    let send = guard.call(
        "hex_app_action",
        args(json!({"app": "messages", "intent": "send",
            "params": {"to": "+49176", "text": "hi"}})),
    );
    assert_eq!(error_code(&send).as_deref(), Some("NEEDS_CONFIRMATION"));
    assert_eq!(ran.lock().unwrap().len(), 2);
}
