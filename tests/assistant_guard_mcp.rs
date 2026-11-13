//! `agent-desk assistant mcp --spec …` (the assistant's desktop guard) over stdio,
//! in front of a fake Cua Driver (its own tool names and shapes) that records
//! what reaches it: refused calls never do, the guard's adapter speaks Cua's
//! tools, and two guard processes agree on who holds a window.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};

const FRONT: &str = "w-18638"; // in front in the recorded list-windows
const SETTINGS: &str = "w-18628";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assistant/guard")
}

struct Guard {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
}

impl Guard {
    fn start(spec: &Value, log: &Path, support: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_agent-desk"))
            .args(["assistant", "mcp", "--spec", &spec.to_string()])
            .env("FAKE_CUA_DRIVER_LOG", log)
            .env("AGENT_DESK_APPLICATION_SUPPORT_DIR", support)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut guard = Self {
            child,
            stdin,
            lines,
            next_id: 0,
        };
        let hello = guard.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "agent-desk-test", "version": "1"}}),
        );
        assert_eq!(hello["result"]["serverInfo"]["name"], "desktop");
        assert!(
            hello["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("Agent-Desk guards these tools")
        );
        guard.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        guard
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("an MCP response");
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("stdout must carry only JSON-RPC: {error}: {line}"));
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// The tool's result: `(is_error, the guard's envelope)`.
    fn call(&mut self, tool: &str, arguments: Value) -> (bool, Value) {
        let response = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &response["result"];
        // A snapshot's text is its outline; the envelope is its structured
        // content (and the text of everything else).
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let envelope = match &result["structuredContent"] {
            Value::Null => serde_json::from_str(text).unwrap_or(Value::Null),
            structured => structured.clone(),
        };
        (result["isError"] == true, envelope)
    }

    fn stop(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

fn reached(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn calls(log: &Path) -> Vec<String> {
    reached(log)
        .iter()
        .filter_map(|entry| entry["call"].as_str().map(str::to_owned))
        .filter(|call| call != "list_windows" && call != "list_apps")
        .collect()
}

/// Batches, scripts, menus, and checks reach Cua Driver as its own tools,
/// each in the task's window; what the guard refuses never does.
#[test]
fn batches_scripts_menus_and_checks_reach_the_driver_in_the_tasks_window() {
    let root = std::env::temp_dir().join(format!("agent-desk-guard-steps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let spec = json!({
        "task": "task-steps", "lane": "background", "physical_input": false,
        "cua_driver": fixtures().join("fake-cua-driver.py"), "state_dir": root.join("state"),
        "windows": "driver", "autonomy": "smart",
    });
    let log = root.join("steps.jsonl");
    let mut guard = Guard::start(&spec, &log, &root.join("support"));
    let tools = guard.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for name in [
        "desktop_run_steps",
        "desktop_run_script",
        "desktop_menu",
        "desktop_verify",
        "hex_skills",
        "hex_skill_record",
        "hex_skill_replay",
    ] {
        assert!(names.contains(&name), "{name} isn't offered: {names:?}");
    }
    assert!(
        !names.contains(&"desktop_screen"),
        "a job without full control"
    );

    // In the person's window nothing runs.
    let (refused, envelope) = guard.call(
        "desktop_run_steps",
        json!({"window_id": FRONT, "steps": [{"action": "press", "combo": "return"}]}),
    );
    assert!(refused);
    assert_eq!(envelope["error"]["code"], "NOT_YOUR_WINDOW");
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));

    let (refused, claimed) = guard.call("desktop_claim_window", json!({"window_id": SETTINGS}));
    assert!(!refused, "{claimed}");
    let (_, snapshot) = guard.call("desktop_snapshot", json!({"window_id": SETTINGS}));
    let ok_button = snapshot["data"]["tree"][1]["ref_id"].clone();
    let (refused, ran) = guard.call(
        "desktop_run_steps",
        json!({"window_id": SETTINGS, "observe": false, "steps": [
            {"action": "click", "ref_id": ok_button},
            {"action": "type", "role": "textfield", "name": "Name", "text": "Ada"},
        ]}),
    );
    assert!(!refused, "{ran}");
    let (refused, verified) = guard.call(
        "desktop_verify",
        json!({"window_id": SETTINGS, "expect": [{"role": "button", "name": "OK"}]}),
    );
    assert!(!refused, "{verified}");
    assert_eq!(verified["data"]["satisfied"], true);
    let (refused, menu) = guard.call(
        "desktop_menu",
        json!({"window_id": SETTINGS, "path": ["Darstellung", "Alle anzeigen"]}),
    );
    assert!(!refused, "{menu}");
    let (refused, scripted) = guard.call(
        "desktop_run_script",
        json!({"window_id": SETTINGS, "script":
            "const read = desk.snapshot(); desk.click(read.tree[1].ref_id); return read.app;"}),
    );
    assert!(!refused, "{scripted}");
    assert_eq!(
        calls(&log),
        [
            "get_window_state",
            "run_steps",
            "verify_state",
            "invoke_menu",
            "get_window_state",
            "click"
        ]
    );
    let entries = reached(&log);
    let batch = entries
        .iter()
        .find(|entry| entry["call"] == "run_steps")
        .unwrap();
    assert_eq!(batch["arguments"]["window_id"], 18628);
    assert_eq!(
        batch["arguments"]["steps"][0]["args"]["element_token"],
        "s00000001:1"
    );
    assert_eq!(batch["arguments"]["steps"][1]["args"]["name"], "Name");
    let menu = entries
        .iter()
        .find(|entry| entry["call"] == "invoke_menu")
        .unwrap();
    assert_eq!(menu["arguments"]["delivery_mode"], "background");
    guard.stop();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_guard_forwards_only_what_the_lane_allows() {
    let root = std::env::temp_dir().join(format!("agent-desk-guard-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fake = fixtures().join("fake-cua-driver.py");
    let spec = |task: &str| {
        json!({
            "task": task, "lane": "background",
            "physical_input": false, "blocked_apps": ["1Password"],
            "cua_driver": fake, "state_dir": root.join("state"),
            // The stand-in's recorded windows, not this Mac's.
            "windows": "driver",
        })
    };
    let log_a = root.join("a.jsonl");
    let mut a = Guard::start(&spec("task-a"), &log_a, &root.join("support"));

    let tools = a.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"desktop_claim_window"));
    assert!(names.contains(&"desktop_snapshot"));
    assert!(!names.contains(&"desktop_batch"));
    assert!(!names.contains(&"desktop_mouse_click"));
    // Cua Driver runs as `mcp --direct`, its telemetry and update checks off.
    let started = reached(&log_a)
        .into_iter()
        .find(|entry| entry["argv"] == json!(["mcp", "--direct"]))
        .expect("Cua Driver started as `mcp --direct`");
    assert_eq!(started["env"]["CUA_DRIVER_RS_UPDATE_CHECK"], "0");
    assert_eq!(started["env"]["CUA_DRIVER_RS_TELEMETRY_ENABLED"], "0");
    assert_eq!(started["env"]["DO_NOT_TRACK"], "1");

    // The window in front is the person's: refused, and never forwarded.
    let (refused, envelope) = a.call("desktop_snapshot", json!({"window_id": FRONT}));
    assert!(refused);
    assert_eq!(envelope["error"]["code"], "NOT_YOUR_WINDOW");
    assert_eq!(
        envelope["error"]["disposition"]["delivery"],
        "not_delivered"
    );
    let (refused, envelope) = a.call("desktop_click", json!({"ref_id": "@sother:e1"}));
    assert!(refused);
    assert_eq!(envelope["error"]["code"], "UNKNOWN_REF");
    let (refused, _) = a.call("desktop_batch", json!({"commands_json": "[]"}));
    assert!(refused);
    assert!(calls(&log_a).is_empty(), "{:?}", calls(&log_a));

    // A free window, claimed, is the task's to read and operate.
    let (refused, claimed) = a.call("desktop_claim_window", json!({"window_id": SETTINGS}));
    assert!(!refused, "{claimed}");
    let (refused, snapshot) = a.call("desktop_snapshot", json!({"window_id": SETTINGS}));
    assert!(!refused, "{snapshot}");
    // The adapter's refs (`@<snapshot>:eN`) are Cua's element tokens.
    let reference = snapshot["data"]["tree"][1]["ref_id"].clone();
    assert_eq!(reference, "@s00000001:e1");
    let (refused, clicked) = a.call("desktop_click", json!({"ref_id": reference}));
    assert!(!refused, "{clicked}");
    assert_eq!(calls(&log_a), ["get_window_state", "click"]);
    let click = reached(&log_a)
        .into_iter()
        .find(|entry| entry["call"] == "click")
        .unwrap();
    assert_eq!(click["arguments"]["element_token"], "s00000001:1");
    assert_eq!(click["arguments"]["window_id"], 18628);

    // Another task's guard, in its own process, can't have it.
    let log_b = root.join("b.jsonl");
    let mut b = Guard::start(&spec("task-b"), &log_b, &root.join("support"));
    let (refused, envelope) = b.call("desktop_claim_window", json!({"window_id": SETTINGS}));
    assert!(refused);
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("another task")
    );
    let (refused, _) = b.call("desktop_snapshot", json!({"window_id": SETTINGS}));
    assert!(refused);
    assert!(calls(&log_b).is_empty());

    // When the first task's guard ends, the window is free again.
    a.stop();
    let (refused, claimed) = b.call("desktop_claim_window", json!({"window_id": SETTINGS}));
    assert!(!refused, "{claimed}");
    b.stop();
    let _ = std::fs::remove_dir_all(&root);
}
