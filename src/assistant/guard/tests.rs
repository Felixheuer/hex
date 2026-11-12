//! The guard's tests: leases across processes, refs mapped to windows, each
//! lane's refusals, the input policy, pausing, and agent-desktop's replies
//! passed through as they came (recorded in `tests/fixtures/assistant/guard`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::child::{self, Desktop};
use super::policy::{self, Kind, Target};
use super::proxy::{Config, Guard, RESUMED};
use super::screen::{App, Screen};
use super::state::{Acquire, Store};
use super::{TaskSpec, WireSpec};
use crate::assistant::backends::McpServer;
use crate::assistant::types::{Lane, TaskWindow};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/assistant/guard")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A folder of its own under the system's temporary folder.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "hex-guard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> Store {
        Store::new(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Another live process, to hold leases as if it were another task's guard.
struct Other(Child);

impl Other {
    fn new() -> Self {
        Self(Command::new("/bin/sleep").arg("60").spawn().unwrap())
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Other {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn dead_pid() -> u32 {
    let mut child = Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

// ── agent-desktop, faked ────────────────────────────────────────────────

#[derive(Default)]
struct FakeState {
    windows: Vec<Value>,
    calls: Vec<(String, Map<String, Value>)>,
    snapshots: u32,
    /// Times the guard looked at the windows on screen.
    looks: u32,
    apps: Vec<App>,
    running: Vec<PathBuf>,
    on_launch: Option<Value>,
    replies: HashMap<String, Value>,
}

/// agent-desktop answering from recorded windows; every call is kept.
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);

impl Fake {
    /// The recorded `list-windows`.
    fn recorded() -> Self {
        let fake = Self::default();
        fake.state().windows = fixture("list-windows.json")["data"]
            .as_array()
            .unwrap()
            .clone();
        fake
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }

    /// The tools that reached agent-desktop, less the guard's own lookups.
    fn forwarded(&self) -> Vec<String> {
        self.state()
            .calls
            .iter()
            .map(|(tool, _)| tool.clone())
            .filter(|tool| tool != "desktop_list_windows" && tool != "desktop_list_apps")
            .collect()
    }

    fn last_args(&self, tool: &str) -> Map<String, Value> {
        self.state()
            .calls
            .iter()
            .rev()
            .find(|(name, _)| name == tool)
            .map(|(_, args)| args.clone())
            .unwrap()
    }

    fn focus(&self, id: &str) {
        for window in &mut self.state().windows {
            window["is_focused"] = json!(window["id"] == id);
        }
    }
}

fn ok(command: &str, data: Value) -> Value {
    let envelope = json!({"version": "2.6", "ok": true, "command": command, "data": data});
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": false,
    })
}

impl Screen for Fake {
    fn windows(&mut self) -> Vec<child::Window> {
        let mut state = self.state();
        state.looks += 1;
        child::windows(&Value::Array(state.windows.clone()))
    }

    fn installed_apps(&mut self) -> Vec<App> {
        self.state().apps.clone()
    }

    fn running_apps(&mut self) -> Vec<PathBuf> {
        self.state().running.clone()
    }
}

impl Desktop for Fake {
    fn tools(&mut self) -> Result<Vec<Value>, String> {
        let result = fixture("mcp-tools.json");
        Ok(result["tools"].as_array().cloned().unwrap_or_default())
    }

    fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
        let mut state = self.state();
        state.calls.push((tool.to_owned(), args.clone()));
        if let Some(reply) = state.replies.get(tool) {
            return Ok(reply.clone());
        }
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(match tool {
            "desktop_list_windows" => ok("list-windows", Value::Array(state.windows.clone())),
            "desktop_list_apps" => ok(
                "list-apps",
                json!({"apps": [
                    {"name": "1Password 7", "bundle_id": "com.agilebits.onepassword7", "pid": 5},
                    {"name": "Ghostty", "bundle_id": "com.mitchellh.ghostty", "pid": 697},
                ]}),
            ),
            "desktop_snapshot" | "desktop_find" => {
                state.snapshots += 1;
                let snapshot = match text("root") {
                    Some(root) => policy::snapshot_of(&root).unwrap(),
                    None => format!("sfake{}", state.snapshots),
                };
                let mut data = json!({
                    "app": text("app").unwrap_or_default(),
                    "snapshot_id": snapshot,
                    "tree": {"role": "window", "children": [
                        {"role": "button", "name": "OK", "ref_id": format!("@{snapshot}:e1")}
                    ]},
                });
                if let Some(id) = text("window_id") {
                    data["window"] = json!({"id": id, "title": ""});
                    let app = state
                        .windows
                        .iter()
                        .find(|window| window["id"] == id.as_str())
                        .and_then(|window| window["app_name"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    data["app"] = json!(app);
                }
                ok("snapshot", data)
            }
            "desktop_launch" => {
                let pid = state
                    .on_launch
                    .as_ref()
                    .and_then(|window| window["pid"].as_i64())
                    .unwrap_or(4242);
                if let Some(window) = state.on_launch.take() {
                    state.windows.push(window);
                }
                ok("launch", json!({"app": text("app"), "pid": pid}))
            }
            other => ok(
                &other.trim_start_matches("desktop_").replace('_', "-"),
                json!({"disposition": {"delivery": "delivered_verified", "retry": "unsafe"}}),
            ),
        })
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn error_code(result: &Value) -> Option<String> {
    let envelope = child::envelope(result)?;
    envelope["error"]["code"].as_str().map(str::to_owned)
}

fn message(result: &Value) -> String {
    child::envelope(result).unwrap()["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn notes(result: &Value) -> Vec<String> {
    result["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|block| block["text"].as_str())
        .filter(|text| text.starts_with("Hex: "))
        .map(str::to_owned)
        .collect()
}

const FRONT: &str = "w-18638"; // Hex Test, in front
const CITRIX: &str = "w-14569";
const SETTINGS: &str = "w-18628";
const GHOSTTY: &str = "w-18108";

fn here(scratch: &Scratch, fake: &Fake, window: &str, app: &str, physical: bool) -> Guard<Fake> {
    let mut guard = Guard::new(
        Config {
            task: "here-task".into(),
            lane: Lane::Here,
            here_window: Some(TaskWindow {
                app: app.into(),
                window_id: Some(window.into()),
            }),
            physical_input: physical,
            blocked_apps: vec!["1Password".into(), "com.agilebits.onepassword7".into()],
        },
        scratch.store(),
        std::process::id(),
        fake.clone(),
        Box::new(fake.clone()),
    );
    // The tests change the windows between calls; every check looks again.
    guard.fresh_for = Duration::ZERO;
    guard
}

fn background(scratch: &Scratch, fake: &Fake, task: &str, physical: bool) -> Guard<Fake> {
    let mut guard = Guard::new(
        Config {
            task: task.into(),
            lane: Lane::Background,
            here_window: None,
            physical_input: physical,
            blocked_apps: vec!["Ghostty".into()],
        },
        scratch.store(),
        std::process::id(),
        fake.clone(),
        Box::new(fake.clone()),
    );
    guard.poll = Duration::from_millis(20);
    guard.window_wait = Duration::from_millis(200);
    guard.fresh_for = Duration::ZERO;
    guard
}

// ── leases ──────────────────────────────────────────────────────────────

#[test]
fn a_window_goes_to_one_process_and_the_other_is_told_whose_it_is() {
    let scratch = Scratch::new();
    let other = Other::new();
    // Two guards' views of the same folder, as two processes would have.
    let ours = scratch.store();
    let theirs = scratch.store();
    let me = std::process::id();
    assert_eq!(
        ours.acquire("a", me, "w-1", "TextEdit", 1),
        Acquire::Acquired
    );
    assert_eq!(ours.acquire("a", me, "w-1", "TextEdit", 2), Acquire::Held);
    assert_eq!(
        theirs.acquire("b", other.pid(), "w-1", "TextEdit", 3),
        Acquire::Taken("a".into())
    );
    assert_eq!(
        theirs.acquire("b", other.pid(), "w-2", "Notes", 4),
        Acquire::Acquired
    );
    assert_eq!(ours.holder("w-2").unwrap().task, "b");
    let leases = ours.leases();
    assert_eq!(
        leases
            .iter()
            .map(|lease| (&*lease.task, &*lease.window_id))
            .collect::<Vec<_>>(),
        [("a", "w-1"), ("b", "w-2")]
    );
    // The lease file is the window's own, named for it.
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(scratch.0.join("leases/w-1.json")).unwrap())
            .unwrap();
    assert_eq!(file["task"], "a");
    assert_eq!(file["pid"], me);
    theirs.release_all("b");
    assert!(ours.holder("w-2").is_none());
    assert_eq!(ours.held_by("a").len(), 1);
}

#[test]
fn a_lease_whose_process_died_lapses_and_is_taken_over() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let dead = dead_pid();
    assert_eq!(
        store.acquire("gone", dead, "w-9", "Mail", 1),
        Acquire::Acquired
    );
    assert!(store.holder("w-9").is_none(), "a dead holder holds nothing");
    assert!(store.leases().is_empty());
    let me = std::process::id();
    assert_eq!(
        store.acquire("new", me, "w-9", "Mail", 2),
        Acquire::Acquired
    );
    assert_eq!(store.holder("w-9").unwrap().task, "new");
    // A task's own lease from an earlier guard process is adopted.
    assert_eq!(
        store.acquire("again", dead, "w-10", "Mail", 3),
        Acquire::Acquired
    );
    assert_eq!(
        store.acquire("again", me, "w-10", "Mail", 4),
        Acquire::Acquired
    );
    assert_eq!(store.holder("w-10").unwrap().pid, me);
    // Sweeping at startup removes what lapsed.
    assert_eq!(
        store.acquire("gone", dead, "w-11", "Mail", 5),
        Acquire::Acquired
    );
    store.sweep();
    assert!(!scratch.0.join("leases/w-11.json").exists());
    assert!(scratch.0.join("leases/w-9.json").exists());
}

#[test]
fn leases_are_listed_as_task_windows() {
    let scratch = Scratch::new();
    let store = scratch.store();
    store.acquire("a", std::process::id(), "w-5", "Notes", 7);
    assert_eq!(
        super::leases_in(&store),
        [super::Lease {
            task: "a".into(),
            window: TaskWindow {
                app: "Notes".into(),
                window_id: Some("w-5".into())
            },
            since_ms: 7
        }]
    );
}

// ── tools and refs ──────────────────────────────────────────────────────

#[test]
fn each_tool_is_sorted_by_what_it_does() {
    let none = Map::new();
    assert_eq!(policy::kind("desktop_list_windows", &none), Kind::Free);
    assert_eq!(policy::kind("desktop_snapshot", &none), Kind::Observe);
    assert_eq!(policy::kind("desktop_click", &none), Kind::Act);
    assert_eq!(policy::kind("desktop_scroll_to", &none), Kind::Act);
    assert_eq!(policy::kind("desktop_mouse_click", &none), Kind::Physical);
    assert_eq!(policy::kind("desktop_double_click", &none), Kind::Physical);
    assert_eq!(
        policy::kind("desktop_wait", &args(json!({"ms": 100}))),
        Kind::Free
    );
    assert_eq!(
        policy::kind("desktop_wait", &args(json!({"element": "@s1:e2"}))),
        Kind::Observe
    );
    for refused in [
        "desktop_batch",
        "desktop_key_down",
        "desktop_session_start",
        "desktop_permissions",
        "desktop_open_system_surface",
        "desktop_notification_action",
        "shell",
    ] {
        assert!(
            matches!(policy::kind(refused, &none), Kind::Refused(_)),
            "{refused}"
        );
    }
}

#[test]
fn a_call_points_at_its_window_ref_app_or_point() {
    let targets = |tool: &str, value: Value| policy::targets(tool, &args(value));
    assert_eq!(
        targets("desktop_click", json!({"ref_id": "@s8f3k2p9:e5"})),
        Ok(vec![Target::Ref("s8f3k2p9".into())])
    );
    assert_eq!(
        targets("desktop_snapshot", json!({"root": "@s1:e3", "app": "Mail"})),
        Ok(vec![Target::Ref("s1".into())])
    );
    assert_eq!(
        targets(
            "desktop_snapshot",
            json!({"window_id": "w-4", "app": "Mail"})
        ),
        Ok(vec![Target::Window("w-4".into())])
    );
    assert_eq!(
        targets(
            "desktop_press",
            json!({"combo": "cmd+s", "app": "TextEdit"})
        ),
        Ok(vec![Target::App("TextEdit".into())])
    );
    assert_eq!(
        targets("desktop_press", json!({"combo": "return"})),
        Ok(vec![Target::Focused])
    );
    assert_eq!(
        targets(
            "desktop_drag",
            json!({"from": "@s1:e1", "to_xy": "=-100,50"})
        ),
        Ok(vec![Target::Ref("s1".into()), Target::Point(-100.0, 50.0)])
    );
    assert!(targets("desktop_click", json!({"ref_id": "@e5"})).is_err());
    assert!(targets("desktop_screenshot", json!({"screen": 0})).is_err());
    assert!(targets("desktop_snapshot", json!({"surface": "dock"})).is_err());
}

#[test]
fn refs_are_found_in_recorded_snapshots() {
    let mut found = Vec::new();
    policy::snapshot_ids(&fixture("snapshot-dock.json")["data"], &mut found);
    assert_eq!(found, ["s3af3jsjbtb05e"]);
    let window = fixture("snapshot-window.json");
    assert_eq!(window["data"]["window"]["id"], "w-18722");
    assert_eq!(
        policy::app_key("System\u{ad}einstellungen"),
        "systemeinstellungen"
    );
}

#[test]
fn refs_map_to_the_window_their_snapshot_was_taken_of() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(error_code(&snapshot), None);
    let reference = child::data(&snapshot).unwrap()["tree"]["children"][0]["ref_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let clicked = guard.call("desktop_click", args(json!({"ref_id": reference})));
    assert_eq!(error_code(&clicked), None);
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_click"]);
    // A drill keeps its snapshot's window.
    let drilled = guard.call("desktop_snapshot", args(json!({"root": reference})));
    assert_eq!(error_code(&drilled), None);
    // A ref from a snapshot taken elsewhere is unknown.
    let stranger = guard.call("desktop_click", args(json!({"ref_id": "@selsewhere:e1"})));
    assert_eq!(error_code(&stranger).as_deref(), Some("UNKNOWN_REF"));
}

// ── lanes ───────────────────────────────────────────────────────────────

#[test]
fn a_here_task_works_only_in_the_window_that_was_in_front() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    let mine = guard.call("desktop_snapshot", args(json!({"app": "Hex Test"})));
    assert_eq!(error_code(&mine), None);
    // `--app` resolved to its one window, which the guard names.
    assert_eq!(fake.last_args("desktop_snapshot")["window_id"], FRONT);
    assert!(fake.last_args("desktop_snapshot").get("app").is_none());
    let unnamed = guard.call("desktop_snapshot", args(json!({})));
    assert_eq!(error_code(&unnamed), None);
    let reference = child::data(&unnamed).unwrap()["tree"]["children"][0]["ref_id"].clone();
    assert_eq!(
        error_code(&guard.call("desktop_click", args(json!({"ref_id": reference})))),
        None
    );
    assert_eq!(
        error_code(&guard.call("desktop_press", args(json!({"combo": "cmd+f"})))),
        None
    );
    for (tool, call) in [
        ("desktop_snapshot", json!({"window_id": CITRIX})),
        ("desktop_screenshot", json!({"app": "Ghostty"})),
        ("desktop_launch", json!({"app": "TextEdit"})),
        ("desktop_claim_window", json!({"window_id": SETTINGS})),
        ("desktop_minimize", json!({"window_id": SETTINGS})),
    ] {
        let result = guard.call(tool, args(call));
        assert!(result["isError"] == true, "{tool}: {result}");
    }
    let other = guard.call("desktop_snapshot", args(json!({"window_id": CITRIX})));
    assert!(
        message(&other).contains("belongs to the person"),
        "{}",
        message(&other)
    );
    assert!(message(&other).contains("w-18638 (Hex Test)"));
    assert_eq!(
        fake.forwarded(),
        [
            "desktop_snapshot",
            "desktop_snapshot",
            "desktop_click",
            "desktop_press"
        ]
    );
    assert_eq!(scratch.store().holder(FRONT).unwrap().task, "here-task");
}

#[test]
fn a_background_task_never_acts_in_the_window_in_front() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    let claim = guard.call("desktop_claim_window", args(json!({"window_id": FRONT})));
    assert_eq!(error_code(&claim).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert!(message(&claim).contains("in front of the person"));
    let unnamed = guard.call("desktop_press", args(json!({"combo": "return"})));
    assert_eq!(error_code(&unnamed).as_deref(), Some("NOT_YOUR_WINDOW"));
    let snapshot = guard.call("desktop_snapshot", args(json!({"window_id": FRONT})));
    assert_eq!(error_code(&snapshot).as_deref(), Some("NOT_YOUR_WINDOW"));
    assert!(message(&snapshot).contains("desktop_launch"));
    // A window it holds that the person brings to front is theirs for now.
    assert_eq!(
        error_code(&guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})))),
        None
    );
    let reference =
        child::data(&guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})))).unwrap()
            ["tree"]["children"][0]["ref_id"]
            .clone();
    fake.focus(SETTINGS);
    let busy = guard.call("desktop_click", args(json!({"ref_id": reference.clone()})));
    assert_eq!(error_code(&busy).as_deref(), Some("PERSON_IN_WINDOW"));
    fake.focus(FRONT);
    assert_eq!(
        error_code(&guard.call("desktop_click", args(json!({"ref_id": reference})))),
        None
    );
    assert_eq!(fake.forwarded(), ["desktop_snapshot", "desktop_click"]);
}

#[test]
fn another_tasks_window_is_refused_before_anything_is_sent() {
    let scratch = Scratch::new();
    let other = Other::new();
    scratch
        .store()
        .acquire("someone-else", other.pid(), CITRIX, "Citrix Viewer", 1);
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    let claim = guard.call("desktop_claim_window", args(json!({"window_id": CITRIX})));
    assert!(message(&claim).contains("another task"));
    let read = guard.call("desktop_snapshot", args(json!({"app": "Citrix Viewer"})));
    assert!(
        message(&read).contains("belongs to another task"),
        "{}",
        message(&read)
    );
    let close = guard.call("desktop_close_app", args(json!({"app": "Citrix Viewer"})));
    assert_eq!(error_code(&close).as_deref(), Some("NOT_YOUR_WINDOW"));
    let point = guard.call("desktop_mouse_click", args(json!({"xy": "0,-100"})));
    assert_eq!(error_code(&point).as_deref(), Some("POLICY_DENIED"));
    assert!(fake.forwarded().is_empty());
}

#[test]
fn a_launch_makes_its_new_window_the_tasks_own() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    fake.state().on_launch = Some(json!({
        "accessible": true, "app_name": "TextEdit", "id": "w-500", "is_focused": false,
        "pid": 900, "title": "Untitled", "visible": true,
        "bounds": {"x": 0.0, "y": 0.0, "width": 500.0, "height": 400.0},
    }));
    let mut guard = background(&scratch, &fake, "bg", false);
    let launched = guard.call("desktop_launch", args(json!({"app": "TextEdit"})));
    assert_eq!(error_code(&launched), None);
    assert!(notes(&launched)[0].contains("w-500 (TextEdit, “Untitled”) is yours now"));
    assert_eq!(scratch.store().holder("w-500").unwrap().task, "bg");
    let typed = guard.call("desktop_snapshot", args(json!({"app": "TextEdit"})));
    assert_eq!(error_code(&typed), None);
    // A window of the app its action opened is its own as well.
    fake.state().windows.push(json!({
        "app_name": "TextEdit", "id": "w-501", "is_focused": false, "pid": 900,
        "title": "Save", "visible": true,
    }));
    let reference = child::data(&typed).unwrap()["tree"]["children"][0]["ref_id"].clone();
    let clicked = guard.call("desktop_click", args(json!({"ref_id": reference})));
    assert!(notes(&clicked)[0].contains("w-501"));
    // But not a window the person opened in front.
    fake.state().windows.push(json!({
        "app_name": "TextEdit", "id": "w-502", "is_focused": true, "pid": 900,
        "title": "Theirs", "visible": true,
    }));
    guard.call("desktop_click", args(json!({"ref_id": "@sfake1:e1"})));
    assert!(scratch.store().holder("w-502").is_none());
    // `activate` would bring it in front of the person.
    let activate = guard.call(
        "desktop_launch",
        args(json!({"app": "Notes", "activate": true})),
    );
    assert_eq!(error_code(&activate).as_deref(), Some("POLICY_DENIED"));
}

#[test]
fn blocked_apps_are_neither_read_nor_operated() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    for call in [
        ("desktop_snapshot", json!({"app": "ghostty"})),
        ("desktop_snapshot", json!({"window_id": GHOSTTY})),
        ("desktop_launch", json!({"app": "Ghostty"})),
        ("desktop_claim_window", json!({"window_id": GHOSTTY})),
        ("desktop_list_windows", json!({"app": "Ghostty"})),
    ] {
        let result = guard.call(call.0, args(call.1));
        assert_eq!(
            error_code(&result).as_deref(),
            Some("BLOCKED_APP"),
            "{}",
            call.0
        );
    }
    // By bundle id too.
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    let result = guard.call("desktop_launch", args(json!({"app": "1Password 7"})));
    assert_eq!(error_code(&result).as_deref(), Some("BLOCKED_APP"));
    assert!(fake.forwarded().is_empty());
}

// ── input ───────────────────────────────────────────────────────────────

#[test]
fn real_input_needs_permission_and_shows_hands_off_while_it_runs() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut headless = here(&scratch, &fake, FRONT, "Hex Test", false);
    let refused = headless.call("desktop_mouse_click", args(json!({"xy": "500,200"})));
    assert_eq!(error_code(&refused).as_deref(), Some("POLICY_DENIED"));
    assert!(
        headless
            .tools()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != "desktop_hover")
    );
    assert!(fake.forwarded().is_empty());

    // While a real click runs, `hands_off` names the task.
    let store = scratch.store();
    let seen = Arc::new(Mutex::new(None));
    let during = seen.clone();
    let watcher = store.clone();
    let fake = Fake::recorded();
    struct Spy(Fake, Arc<Mutex<Option<String>>>, Store);
    impl Desktop for Spy {
        fn tools(&mut self) -> Result<Vec<Value>, String> {
            self.0.tools()
        }
        fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
            if tool == "desktop_mouse_click" {
                *self.1.lock().unwrap() = self.2.hands_off();
            }
            self.0.call(tool, args)
        }
    }
    let mut spied = Guard::new(
        Config {
            task: "here-task".into(),
            lane: Lane::Here,
            here_window: Some(TaskWindow {
                app: "Hex Test".into(),
                window_id: Some(FRONT.into()),
            }),
            physical_input: true,
            blocked_apps: Vec::new(),
        },
        store.clone(),
        std::process::id(),
        Spy(fake.clone(), during, watcher),
        Box::new(fake.clone()),
    );
    assert!(
        spied
            .tools()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "desktop_hover")
    );
    let clicked = spied.call("desktop_mouse_click", args(json!({"xy": "1300,500"})));
    assert_eq!(error_code(&clicked), None);
    assert_eq!(seen.lock().unwrap().as_deref(), Some("here-task"));
    assert_eq!(
        store.hands_off(),
        None,
        "the marker goes when the click is done"
    );
    // A point outside its window is refused even with real input allowed.
    let outside = spied.call("desktop_mouse_click", args(json!({"xy": "-500,-500"})));
    assert_eq!(error_code(&outside).as_deref(), Some("NOT_YOUR_WINDOW"));
}

// ── pause ───────────────────────────────────────────────────────────────

#[test]
fn a_paused_task_waits_and_then_is_told_to_look_again() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    guard.call("desktop_claim_window", args(json!({"window_id": SETTINGS})));
    let store = scratch.store();
    store.set_paused("bg", true, 1);
    // Reads go on while paused.
    let read = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert_eq!(error_code(&read), None);
    let reference = child::data(&read).unwrap()["tree"]["children"][0]["ref_id"].clone();
    let resumer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        store.set_paused("bg", false, 2);
    });
    let started = std::time::Instant::now();
    let held = guard.call("desktop_click", args(json!({"ref_id": reference.clone()})));
    resumer.join().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(140), "it waited");
    assert_eq!(error_code(&held).as_deref(), Some("TAKEN_OVER"));
    assert_eq!(message(&held), RESUMED);
    assert_eq!(
        fake.forwarded(),
        ["desktop_snapshot"],
        "the waiting click wasn't sent"
    );
    // Then it acts again.
    let clicked = guard.call("desktop_click", args(json!({"ref_id": reference.clone()})));
    assert_eq!(error_code(&clicked), None);

    // Taken over and handed back between calls: the next read carries the
    // note, and an action is held back once.
    scratch.store().set_paused("bg", true, 3);
    scratch.store().set_paused("bg", false, 4);
    let read = guard.call("desktop_snapshot", args(json!({"window_id": SETTINGS})));
    assert!(notes(&read)[0].contains("take a fresh snapshot"), "{read}");
    scratch.store().set_paused("bg", true, 5);
    scratch.store().set_paused("bg", false, 6);
    let action = guard.call("desktop_click", args(json!({"ref_id": reference.clone()})));
    assert_eq!(error_code(&action).as_deref(), Some("TAKEN_OVER"));
    let action = guard.call("desktop_click", args(json!({"ref_id": reference})));
    assert_eq!(error_code(&action), None);
}

// ── passthrough ─────────────────────────────────────────────────────────

#[test]
fn agent_desktops_replies_pass_through_as_recorded() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let recorded = fixture("mcp-list-windows-result.json");
    let refused = fixture("mcp-unknown-argument-result.json");
    fake.state()
        .replies
        .insert("desktop_list_windows".into(), recorded.clone());
    fake.state()
        .replies
        .insert("desktop_status".into(), refused.clone());
    let mut guard = here(&scratch, &fake, FRONT, "Hex Test", false);
    let mut listed = guard.call("desktop_list_windows", args(json!({"app": "Ghostty"})));
    // Hex adds one note block; everything agent-desktop said is unchanged.
    let added = listed["content"].as_array_mut().unwrap().pop().unwrap();
    assert!(
        added["text"]
            .as_str()
            .unwrap()
            .starts_with("Hex: this task works only in")
    );
    assert_eq!(listed, recorded);
    assert_eq!(guard.call("desktop_status", Map::new()), refused);
    // And through the MCP model types the server returns.
    let typed: rmcp::model::CallToolResult = serde_json::from_value(recorded.clone()).unwrap();
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(back["content"], recorded["content"]);
    assert_eq!(back["structuredContent"], recorded["structuredContent"]);
    assert_eq!(back["isError"], false);
}

#[test]
fn the_tool_list_keeps_agent_desktops_schemas_and_adds_claim() {
    let scratch = Scratch::new();
    let fake = Fake::recorded();
    let mut guard = background(&scratch, &fake, "bg", false);
    let tools = guard.tools().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for kept in [
        "desktop_snapshot",
        "desktop_click",
        "desktop_launch",
        "desktop_list_windows",
    ] {
        assert!(names.contains(&kept), "{kept}");
    }
    for gone in [
        "desktop_batch",
        "desktop_session_start",
        "desktop_mouse_click",
        "desktop_focus_window",
        "desktop_permissions",
    ] {
        assert!(!names.contains(&gone), "{gone}");
    }
    assert!(names.contains(&super::CLAIM_TOOL));
    let snapshot = tools
        .iter()
        .find(|tool| tool["name"] == "desktop_snapshot")
        .unwrap();
    let original = fixture("mcp-tools.json")["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "desktop_snapshot")
        .cloned()
        .unwrap();
    assert_eq!(*snapshot, original);
    for tool in &tools {
        serde_json::from_value::<rmcp::model::Tool>(tool.clone()).unwrap();
    }
}

// ── the spec ────────────────────────────────────────────────────────────

#[test]
fn a_task_gets_hex_itself_as_its_desktop_server() {
    let spec = TaskSpec {
        task: "c1".into(),
        lane: Lane::Here,
        agent_id: "hex-1".into(),
        session: Some("s1".into()),
        here_window: Some(TaskWindow {
            app: "Mail".into(),
            window_id: Some("w-7".into()),
        }),
        physical_input: true,
        blocked_apps: vec!["1Password".into()],
    };
    let desktop = McpServer {
        command: "/opt/homebrew/bin/agent-desktop".into(),
        args: vec!["mcp".into()],
        path_env: "/opt/homebrew/bin:/usr/bin".into(),
    };
    let server = super::server_for(
        &spec,
        Path::new("/Applications/Hex.app/Contents/MacOS/hex"),
        &desktop,
        Path::new("/Users/me/Library/Application Support/hex/assistant"),
    );
    assert_eq!(
        server.command,
        Path::new("/Applications/Hex.app/Contents/MacOS/hex")
    );
    assert_eq!(server.args[..3], ["assistant", "mcp", "--spec"]);
    assert_eq!(server.path_env, desktop.path_env);
    let wire = WireSpec::parse(&server.args[3]).unwrap();
    assert_eq!(
        serde_json::to_value(&wire).unwrap(),
        json!({
            "task": "c1", "lane": "here", "agent_id": "hex-1", "session": "s1",
            "here_window": {"app": "Mail", "window_id": "w-7"},
            "physical_input": true, "blocked_apps": ["1Password"],
            "agent_desktop": "/opt/homebrew/bin/agent-desktop",
            "state_dir": "/Users/me/Library/Application Support/hex/assistant",
        })
    );
    let minimal = WireSpec::parse(r#"{"task":"t","lane":"background","agent_id":"a"}"#).unwrap();
    assert_eq!(minimal.lane, Lane::Background);
    assert!(!minimal.physical_input);
    // Claude Code gets it under agent-desktop's name, so tool names stay.
    let claude: Value = serde_json::from_str(&server.claude_config()).unwrap();
    assert_eq!(claude["mcpServers"]["agent-desktop"]["args"][1], "mcp");
}

mod round_two;
