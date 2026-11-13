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
    let mut guard = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
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
    // the driver as the running app's own name.
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
    let mut later = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
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

/// Keys sent to window `id` by pid (`hex_window_input` key), as the agent
/// tries before asking for the front.
fn keys_by_pid(guard: &mut Guard<Fake>, id: &str) {
    let keys = guard.call(
        "hex_window_input",
        args(json!({"window_id": id, "action": "key", "text": "cmd+l"})),
    );
    assert_eq!(keys["isError"], false, "{keys}");
}

#[test]
fn focusing_waits_for_background_actions_to_fail_first() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
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
    // Asked again before any key went by pid: still refused.
    let again = guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&again).as_deref(), Some("BACKGROUND_FIRST"));
    // Keys by pid first; asked after that (they didn't land), it goes
    // ahead, briefly.
    keys_by_pid(&mut guard, CITRIX);
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

/// A window brought forward is stored with why and until when, so
/// Agent-Desk shows the desk frame and its tab, and goes with the borrow:
/// given back, or the person's once they took the task over.
#[test]
fn a_borrow_is_stored_until_the_front_goes_back() {
    for taken_over in [false, true] {
        let scratch = Scratch::new();
        let fake = Fake::recorded();
        with_running_apps(&fake);
        let mut guard = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
        guard.config.front_window = true;
        guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
        assert!(scratch.store().borrowed().is_empty());
        guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
        assert!(
            scratch.store().borrowed().is_empty(),
            "a refused focus borrows nothing"
        );
        keys_by_pid(&mut guard, CITRIX);
        guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
        let borrowed = scratch.store().borrowed();
        assert_eq!(borrowed.len(), 1, "{borrowed:?}");
        let borrow = &borrowed[0];
        assert_eq!(borrow.task, guard.config.task);
        assert_eq!(borrow.window_id, CITRIX);
        assert_eq!(borrow.why, crate::assistant::guard::BORROW_KEYS);
        assert_eq!(borrow.until_ms - borrow.at_ms, 20_000);
        if taken_over {
            scratch.store().set_paused(&guard.config.task, true, 1);
        }
        let task = guard.config.task.clone();
        drop(guard);
        assert!(scratch.store().borrowed().is_empty(), "{taken_over}");
        scratch.store().set_paused(&task, false, 2);
    }
}

/// The island announces a borrow before the window comes forward: the
/// borrow is stored before the focus reaches the desktop; a take-over
/// during the announce brings nothing forward and borrows nothing.
#[test]
fn a_borrow_is_announced_before_the_window_comes_forward() {
    for taken_over in [false, true] {
        let scratch = Scratch::new();
        let fake = Fake::recorded();
        with_running_apps(&fake);
        let mut guard = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
        guard.config.front_window = true;
        guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
        guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
        keys_by_pid(&mut guard, CITRIX);
        let store = scratch.store();
        let task = guard.config.task.clone();
        let announced = Arc::new(Mutex::new(None::<bool>));
        let seen = announced.clone();
        let looking = store.clone();
        fake.state().on_call = Some(Arc::new(move |tool: &str| {
            if tool == "desktop_focus_window" {
                *seen.lock().unwrap() = Some(!looking.borrowed().is_empty());
            }
        }));
        guard.announce = Duration::from_millis(300);
        // The person takes over while the island announces it.
        let pauser = taken_over.then(|| {
            let (store, task) = (store.clone(), task.clone());
            std::thread::spawn(move || {
                while store.borrowed().is_empty() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                store.set_paused(&task, true, 1);
            })
        });
        let result = guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
        if let Some(pauser) = pauser {
            pauser.join().unwrap();
        }
        fake.state().on_call = None;
        if taken_over {
            assert!(result.to_string().contains("TAKEN_OVER"), "{result}");
            assert_eq!(*announced.lock().unwrap(), None, "nothing came forward");
            assert!(scratch.store().borrowed().is_empty(), "nothing borrowed");
            scratch.store().set_paused(&task, false, 2);
        } else {
            assert_eq!(
                *announced.lock().unwrap(),
                Some(true),
                "the borrow was stored (and announced) first"
            );
            assert_eq!(scratch.store().borrowed().len(), 1);
        }
    }
}

#[test]
fn any_app_takes_a_url_or_a_script_and_blocked_apps_and_sends_are_checked() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    {
        let mut state = fake.state();
        state.apps.push(app("Ghostty", "com.mitchellh.ghostty"));
    }
    let ran = Arc::new(Mutex::new(Vec::new()));
    let mut guard = background(&scratch, &fake, "bg", false);
    let seen = ran.clone();
    guard.run_action = Box::new(move |command| {
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
    assert!(tools.contains(&"hex_app_capabilities".to_owned()));
    assert!(tools.contains(&"hex_read_window".to_owned()));
    // The real failure: a password in a note, its window unreachable. The
    // note is read by script, without any window.
    let read = guard.call(
        "hex_app_action",
        args(json!({"app": "Notes", "intent": "script",
            "params": {"script": "get plaintext of every note whose name contains \"Kraftstoffnotdienst\""}})),
    );
    assert_eq!(error_code(&read), None, "{read}");
    assert!(ran.lock().unwrap()[0].contains("tell application id \\\"com.apple.Notes\\\""));
    // Any app takes a URL.
    let opened = guard.call(
        "hex_app_action",
        args(json!({"app": "Notes", "intent": "open_url",
            "params": {"url": "notes://showNote?identifier=1"}})),
    );
    assert_eq!(error_code(&opened), None, "{opened}");
    assert_eq!(ran.lock().unwrap().len(), 2);
    // No per-app intents.
    let unknown = guard.call(
        "hex_app_action",
        args(json!({"app": "WhatsApp", "intent": "open_chat", "params": {"phone": "+49176"}})),
    );
    assert_eq!(error_code(&unknown).as_deref(), Some("UNKNOWN_INTENT"));
    // Ghostty is blocked for this task, for actions and for a look at it.
    let blocked = guard.call(
        "hex_app_action",
        args(json!({"app": "ghostty", "intent": "open_url", "params": {"url": "file:///tmp"}})),
    );
    assert_eq!(error_code(&blocked).as_deref(), Some("BLOCKED_APP"));
    let blocked = guard.call("hex_app_capabilities", args(json!({"app": "Ghostty"})));
    assert_eq!(error_code(&blocked).as_deref(), Some("BLOCKED_APP"));
    // Sending by script needs the person's yes.
    let send = guard.call(
        "hex_app_action",
        args(json!({"app": "Notes", "intent": "script",
            "params": {"script": "send note 1"}})),
    );
    assert_eq!(error_code(&send).as_deref(), Some("NEEDS_CONFIRMATION"));
    assert_eq!(ran.lock().unwrap().len(), 2);
}

#[test]
fn what_an_app_offers_comes_from_the_app_itself() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = background(&scratch, &fake, "bg", false);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    guard.discover = Box::new(move |app| {
        seen.lock().unwrap().push(app.path.clone());
        json!({"app": app.name, "scripting": "AppleScript and JXA work"})
    });
    // By name or bundle id, as installed.
    let found = guard.call(
        "hex_app_capabilities",
        args(json!({"app": "com.apple.Notes"})),
    );
    assert_eq!(data(&found)["app"], "Notes", "{found}");
    assert_eq!(
        *asked.lock().unwrap(),
        [std::path::PathBuf::from("/Applications/Notes.app")]
    );
    let missing = guard.call("hex_app_capabilities", args(json!({"app": "Nope"})));
    assert_eq!(error_code(&missing).as_deref(), Some("APP_NOT_FOUND"));
    // A look only: a check that may change nothing still gets it.
    assert!(!super::super::policy::Kind::AppCapabilities.acts());
    assert!(!super::super::policy::Kind::AppCapabilities.needs_desktop());
}

/// Snapshots window `id` through `guard` and clicks its first element.
fn click_in(guard: &mut Guard<Fake>, id: &str) -> Value {
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": id})));
    let Some(data) = child::data(&snapshot) else {
        return snapshot;
    };
    let reference = data["tree"]["children"][0]["ref_id"]
        .as_str()
        .unwrap()
        .to_owned();
    guard.call("desktop_click", args(json!({"ref_id": reference})))
}

#[test]
fn a_desk_window_is_one_task_s_at_a_time_and_a_job_must_claim_it() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let pid = std::process::id();
    // An earlier quick turn left Citrix on the desk.
    assert_eq!(
        scratch
            .store()
            .acquire("desk", pid, CITRIX, "Citrix Viewer", 1),
        Acquire::Acquired
    );
    let mut job = background(&scratch, &fake, "job", false);
    job.desk = Some(pid);
    // A job doesn't act in it, or even read it, without claiming it.
    let read = job.call("desktop_snapshot", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&read).as_deref(), Some("DESK_BUSY"), "{read}");
    assert!(message(&read).contains("desktop_claim_window"));
    // The person's next quick turn acts in it, and borrows it.
    let mut turn = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
    turn.config.open_apps = true;
    turn.desk = Some(pid);
    let clicked = click_in(&mut turn, CITRIX);
    assert_eq!(error_code(&clicked), None, "{clicked}");
    // While the turn has it, the job's claim is refused.
    let claim = job.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&claim).as_deref(), Some("DESK_BUSY"), "{claim}");
    assert!(message(&claim).contains("quick turn"), "{claim}");
    // The turn ends: its borrow comes back, and the job's claim goes
    // through; then it acts there.
    scratch.store().release_all("here-task");
    super::super::desk::return_borrows(&scratch.store(), "here-task");
    let claim = job.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&claim), None, "{claim}");
    assert_eq!(data(&claim)["desk"], true);
    let clicked = click_in(&mut job, CITRIX);
    assert_eq!(error_code(&clicked), None, "{clicked}");
    // A second job is refused while the first has it.
    let mut other = background(&scratch, &fake, "other-job", false);
    other.desk = Some(pid);
    let claim = other.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    assert_eq!(error_code(&claim).as_deref(), Some("DESK_BUSY"), "{claim}");
    // A new quick turn takes it from the job, whose next action is refused.
    let mut next = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
    next.config.task = "next-turn".into();
    next.config.open_apps = true;
    next.desk = Some(pid);
    let clicked = click_in(&mut next, CITRIX);
    assert_eq!(error_code(&clicked), None, "{clicked}");
    let refused = click_in(&mut job, CITRIX);
    assert_eq!(
        error_code(&refused).as_deref(),
        Some("DESK_BUSY"),
        "{refused}"
    );
    // The window stays the desk's throughout.
    assert_eq!(scratch.store().holder(CITRIX).unwrap().task, "desk");
}

/// A borrow for keys says so (`BORROW_KEYS`: "Keyboard input in …" on the
/// frame), and when it ends the front goes back to the app the person was
/// in before; Citrix (pid 34651) took it while borrowed.
#[test]
fn a_borrowed_front_goes_back_to_the_person_s_app() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = here(&scratch, &fake, FRONT, "Agent-Desk Test", false);
    guard.config.front_window = true;
    // The person is in their app (pid 81316, the window in front).
    let front = super::super::proxy::front::fake::Fake::at(81316);
    guard.front = Box::new(front.clone());
    guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    assert!(
        front.given().is_empty(),
        "opening a running app took nothing"
    );
    keys_by_pid(&mut guard, CITRIX);
    let coming = front.clone();
    fake.state().on_call = Some(Arc::new(move |tool: &str| {
        if tool == "desktop_focus_window" {
            coming.set(34651);
        }
    }));
    let focused = guard.call("desktop_focus_window", args(json!({"window_id": CITRIX})));
    fake.state().on_call = None;
    assert_eq!(error_code(&focused), None, "{focused}");
    assert_eq!(front.now(), Some(34651));
    let borrowed = scratch.store().borrowed();
    assert_eq!(borrowed[0].why, crate::assistant::guard::BORROW_KEYS);
    assert_eq!(borrowed[0].app, "Citrix Viewer");
    // The borrow's steps in the window, then the front goes back.
    for _ in 0..4 {
        let pressed = guard.call(
            "desktop_press",
            args(json!({"window_id": CITRIX, "combo": "return"})),
        );
        assert_eq!(pressed["isError"], false, "{pressed}");
    }
    assert_eq!(front.given(), [81316]);
    assert_eq!(front.now(), Some(81316));
    assert!(scratch.store().borrowed().is_empty());
}

/// `hex_open_app` on a running app never leaves it in front: when a step
/// (here the reopen of an app without a window) brought it forward, the
/// person's app gets the front back.
#[test]
fn opening_a_running_app_keeps_the_person_s_app_in_front() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    with_running_apps(&fake);
    let mut guard = background(&scratch, &fake, "bg", false);
    let front = super::super::proxy::front::fake::Fake::at(81316);
    guard.front = Box::new(front.clone());
    let (screen, coming) = (fake.clone(), front.clone());
    guard.reopen = Box::new(move |_app| {
        // Notes shows a window again, and comes forward with it.
        screen.state().windows.push(json!({
            "app_name": "Notes", "id": "w-950", "is_focused": false, "pid": 4200,
            "title": "Notes", "visible": true,
        }));
        coming.set(4200);
    });
    let opened = guard.call("hex_open_app", args(json!({"name": "Notes"})));
    assert_eq!(data(&opened)["windows"][0]["id"], "w-950", "{opened}");
    assert_eq!(front.given(), [81316]);
    assert_eq!(front.now(), Some(81316));
    // A running app's window on screen is taken without anything coming
    // forward, and nothing is given back.
    let front = super::super::proxy::front::fake::Fake::at(81316);
    guard.front = Box::new(front.clone());
    guard.call("hex_open_app", args(json!({"name": "Citrix Viewer"})));
    assert!(front.given().is_empty());
}
