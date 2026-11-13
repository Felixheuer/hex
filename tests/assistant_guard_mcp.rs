//! `agent-desk assistant mcp --spec …` (the assistant's desktop guard) over stdio,
//! in front of a fake agent-desktop that records what reaches it: refused
//! calls never do, and two guard processes agree on who holds a window.
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
            .env("FAKE_AGENT_DESKTOP_LOG", log)
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
        assert_eq!(hello["result"]["serverInfo"]["name"], "agent-desktop");
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

    /// The tool's result: `(is_error, agent-desktop envelope)`.
    fn call(&mut self, tool: &str, arguments: Value) -> (bool, Value) {
        let response = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &response["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        (
            result["isError"] == true,
            serde_json::from_str(text).unwrap_or(Value::Null),
        )
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
        .filter(|call| call != "desktop_list_windows")
        .collect()
}

#[test]
fn the_guard_forwards_only_what_the_lane_allows() {
    let root = std::env::temp_dir().join(format!("hex-guard-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fake = fixtures().join("fake-agent-desktop.py");
    let spec = |task: &str, agent: &str| {
        json!({
            "task": task, "lane": "background", "agent_id": agent, "session": "s1",
            "physical_input": false, "blocked_apps": ["1Password"],
            "agent_desktop": fake, "state_dir": root.join("state"),
            // The stand-in's recorded windows, not this Mac's.
            "windows": "agent_desktop",
        })
    };
    let log_a = root.join("a.jsonl");
    let mut a = Guard::start(&spec("task-a", "hex-1"), &log_a, &root.join("support"));

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
    assert!(
        reached(&log_a)
            .iter()
            .any(|entry| entry["argv"] == json!(["mcp", "--session", "s1", "--agent-id", "hex-1"])),
        "{:?}",
        reached(&log_a)
    );

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
    let reference = snapshot["data"]["tree"]["children"][0]["ref_id"].clone();
    let (refused, _) = a.call("desktop_click", json!({"ref_id": reference}));
    assert!(!refused);
    assert_eq!(calls(&log_a), ["desktop_snapshot", "desktop_click"]);

    // Another task's guard, in its own process, can't have it.
    let log_b = root.join("b.jsonl");
    let mut b = Guard::start(&spec("task-b", "hex-2"), &log_b, &root.join("support"));
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
