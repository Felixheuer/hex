//! Cua Driver behind the guard (`assistant::driver`): its MCP tools
//! (`list_windows`, `get_window_state`, `click` by `element_token`, …)
//! served as the guard's `desktop_*` tools, with agent-desktop's argument
//! names, refs (`@<snapshot>:eN` for Cua's `<snapshot>:N`), and result
//! envelopes (`{ok, command, data | error}`), so the guard's leases, consent,
//! and fallbacks, the prompts, and the overlay work unchanged whichever
//! driver runs.
//!
//! Window ids are CoreGraphics window numbers either way (`w-<number>`).
//! Moving, minimizing, and restoring a window go through Hex's own
//! Accessibility calls (`actions::ax`): Cua Driver refuses frames off the
//! screen, which parking needs. Real pointer tools aren't offered; acting
//! by a window's picture is `hex_window_input`.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::child::Desktop;

/// Snapshots whose elements are kept (refs of older ones are stale).
const KEPT: usize = 12;
/// Most elements a snapshot reads.
const MAX_ELEMENTS: u64 = 400;

/// Cua Driver's MCP server (`inner`) as the guard's desktop.
pub struct Cua<I: Desktop> {
    inner: I,
    /// Window number → its process, from listings.
    pids: HashMap<u32, i64>,
    /// Snapshot id → (pid, window number, its elements), newest last.
    snapshots: VecDeque<(String, i64, u32, Vec<Value>)>,
}

impl<I: Desktop> Cua<I> {
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            pids: HashMap::new(),
            snapshots: VecDeque::new(),
        }
    }

    fn raw(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let args = args.as_object().cloned().unwrap_or_default();
        self.inner.call(tool, &args)
    }

    /// Cua's tool result as data, or its refusal as our envelope.
    fn data(&mut self, command: &str, tool: &str, args: Value) -> Result<Value, Value> {
        let result = self
            .raw(tool, args)
            .map_err(|why| error(command, "DRIVER_FAILED", &why))?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(refused(command, &result));
        }
        Ok(result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| json!({"text": text_of(&result)})))
    }

    /// Every layer-0 window Cua lists (of `app` when given).
    fn windows(&mut self, app: Option<&str>) -> Result<Vec<Value>, Value> {
        let mut args = json!({});
        if let Some(app) = app {
            args["app"] = json!(app);
        }
        let data = self.data("list-windows", "list_windows", args)?;
        let windows: Vec<Value> = data["windows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|window| window["layer"].as_i64().unwrap_or(0) == 0)
            .collect();
        for window in &windows {
            if let (Some(number), Some(pid)) =
                (window["window_id"].as_u64(), window["pid"].as_i64())
            {
                self.pids.insert(number as u32, pid);
            }
        }
        Ok(windows)
    }

    /// The (pid, window number) a call names by `window_id` or `app`.
    fn target(&mut self, command: &str, args: &Map<String, Value>) -> Result<(i64, u32), Value> {
        if let Some(number) = text(args, "window_id")
            .as_deref()
            .and_then(crate::assistant::computer::window_number)
        {
            if let Some(pid) = self.pids.get(&number) {
                return Ok((*pid, number));
            }
            self.windows(None)?;
            return self
                .pids
                .get(&number)
                .map(|pid| (*pid, number))
                .ok_or_else(|| {
                    error(
                        command,
                        "WINDOW_NOT_FOUND",
                        &format!("There's no window w-{number}."),
                    )
                });
        }
        if let Some(app) = text(args, "app") {
            let windows = self.windows(Some(&app))?;
            let pick = windows
                .iter()
                .find(|window| window["is_on_screen"] == true)
                .or(windows.first())
                .ok_or_else(|| {
                    error(
                        command,
                        "WINDOW_NOT_FOUND",
                        &format!("{app} has no window."),
                    )
                })?;
            return Ok((
                pick["pid"].as_i64().unwrap_or_default(),
                pick["window_id"].as_u64().unwrap_or_default() as u32,
            ));
        }
        Err(error(
            command,
            "INVALID_ARGS",
            "Name the window with window_id (or the app with app).",
        ))
    }

    /// The snapshot a ref is from and its token for Cua.
    fn element(
        &self,
        command: &str,
        reference: &str,
    ) -> Result<(String, i64, u32, Option<Value>), Value> {
        let (snapshot, row) = token(reference).ok_or_else(|| {
            error(
                command,
                "INVALID_ARGS",
                &format!("“{reference}” isn't a ref (@<snapshot>:eN)."),
            )
        })?;
        let (_, pid, number, elements) = self
            .snapshots
            .iter()
            .find(|(id, ..)| *id == snapshot)
            .ok_or_else(|| {
                error(
                    command,
                    "STALE_REF",
                    "That ref is from an old snapshot; take a new snapshot of the window.",
                )
            })?;
        let found = elements
            .iter()
            .find(|element| element["element_index"].as_u64() == Some(row))
            .cloned();
        Ok((format!("{snapshot}:{row}"), *pid, *number, found))
    }

    fn remember(&mut self, snapshot: &str, pid: i64, number: u32, elements: Vec<Value>) {
        self.snapshots.retain(|(id, ..)| id != snapshot);
        self.snapshots
            .push_back((snapshot.to_owned(), pid, number, elements));
        while self.snapshots.len() > KEPT {
            self.snapshots.pop_front();
        }
    }

    fn snapshot(
        &mut self,
        command: &str,
        args: &Map<String, Value>,
        query: Option<String>,
    ) -> Result<Value, Value> {
        let (pid, number) = self.target(command, args)?;
        let mut ask = json!({
            "pid": pid,
            "window_id": number,
            "include_screenshot": false,
            "tree_format": "elements",
            "max_elements": MAX_ELEMENTS,
        });
        if let Some(depth) = args.get("max_depth").and_then(Value::as_u64) {
            ask["max_depth"] = json!(depth);
        }
        if let Some(query) = &query {
            ask["query"] = json!(query);
        }
        let data = self.data(command, "get_window_state", ask)?;
        let snapshot = data["snapshot_id"].as_str().unwrap_or_default().to_owned();
        let mut elements = data["elements"].as_array().cloned().unwrap_or_default();
        for element in &mut elements {
            redact_secure(element);
        }
        self.remember(&snapshot, pid, number, elements.clone());
        // Drill into one element's subtree (`root`), or only what acts.
        if let Some(root) = text(args, "root").and_then(|root| token(&root)) {
            elements = subtree(&elements, root.1);
        }
        if args.get("interactive_only").and_then(Value::as_bool) == Some(true) {
            elements.retain(|element| {
                element["actions"]
                    .as_array()
                    .is_some_and(|actions| !actions.is_empty())
            });
        }
        if let Some(role) = text(args, "role") {
            let role = role.to_lowercase();
            elements.retain(|element| {
                element["role"]
                    .as_str()
                    .is_some_and(|found| found.to_lowercase().contains(&role))
            });
        }
        let tree: Vec<Value> = elements
            .iter()
            .map(|element| ours(&snapshot, element))
            .collect();
        let outline = outline(&tree);
        let envelope = envelope_ok(
            command,
            json!({
                "snapshot_id": snapshot,
                "app": data["app_name"],
                "window": {"id": format!("w-{number}"), "title": data["window_title"]},
                "truncated": data["truncated"],
                "tree": tree,
            }),
        );
        Ok(json!({
            "content": [{"type": "text", "text": format!(
                "snapshot {snapshot} of w-{number} ({}): act on a ref such as @{snapshot}:e1\n{outline}",
                data["app_name"].as_str().unwrap_or_default()
            )}],
            "structuredContent": envelope,
            "isError": false,
        }))
    }

    /// An action on a ref through Cua's `tool` with `extra` arguments.
    fn on_ref(
        &mut self,
        command: &str,
        tool: &str,
        args: &Map<String, Value>,
        extra: Value,
    ) -> Result<Value, Value> {
        let reference = text(args, "ref_id").ok_or_else(|| {
            error(
                command,
                "INVALID_ARGS",
                "This needs ref_id from a snapshot.",
            )
        })?;
        let (token, pid, number, _) = self.element(command, &reference)?;
        let mut ask = json!({"pid": pid, "window_id": number, "element_token": token});
        if let Some(extra) = extra.as_object() {
            for (key, value) in extra {
                ask[key] = value.clone();
            }
        }
        let data = self.data(command, tool, ask)?;
        Ok(result_ok(command, outcome(&data)))
    }

    fn call_inner(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let command = tool
            .strip_prefix("desktop_")
            .unwrap_or(tool)
            .replace('_', "-");
        let command = command.as_str();
        match tool {
            "desktop_status" => {
                let permissions = if cfg!(test) {
                    crate::assistant::computer::Permissions::default()
                } else {
                    crate::assistant::computer::permissions_now()
                };
                let state = |granted: bool| if granted { "granted" } else { "denied" };
                Ok(result_ok(
                    "status",
                    json!({
                        "driver": "cua-driver",
                        "version": crate::assistant::driver::cua::VERSION,
                        "permissions": {
                            "accessibility": {"state": state(permissions.accessibility)},
                            "screen_recording": {"state": state(permissions.screen_recording)},
                        },
                    }),
                ))
            }
            "desktop_version" => Ok(result_ok(
                command,
                json!({"version": format!("cua-driver {}", crate::assistant::driver::cua::VERSION)}),
            )),
            "desktop_list_windows" => {
                let app = text(args, "app");
                let windows = self.windows(app.as_deref())?;
                let front = frontmost();
                let top = |pid: i64| {
                    windows
                        .iter()
                        .filter(|window| window["pid"].as_i64() == Some(pid))
                        .filter_map(|window| window["z_index"].as_i64())
                        .max()
                };
                let listed: Vec<Value> = windows
                    .iter()
                    .map(|window| {
                        let pid = window["pid"].as_i64().unwrap_or_default();
                        let bounds = &window["bounds"];
                        json!({
                            "id": format!("w-{}", window["window_id"].as_u64().unwrap_or_default()),
                            "app_name": window["app_name"],
                            "title": window["title"],
                            "pid": pid,
                            "is_focused": front == Some(pid)
                                && window["z_index"].as_i64().is_some()
                                && window["z_index"].as_i64() == top(pid),
                            "visible": window["is_on_screen"] == true,
                            "bounds": {"x": bounds["x"], "y": bounds["y"], "width": bounds["width"], "height": bounds["height"]},
                        })
                    })
                    .collect();
                Ok(result_ok(command, Value::Array(listed)))
            }
            "desktop_list_apps" => {
                let data = self.data(command, "list_apps", json!({}))?;
                let apps: Vec<Value> = data["apps"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|app| app["running"] == true)
                    .map(|app| json!({"name": app["name"], "bundle_id": app["bundle_id"], "pid": app["pid"]}))
                    .collect();
                Ok(result_ok(command, json!({"apps": apps})))
            }
            "desktop_snapshot" => self.snapshot(command, args, None),
            "desktop_find" => {
                let query = ["name", "text", "value", "description"]
                    .iter()
                    .find_map(|key| text(args, key));
                self.snapshot(command, args, query)
            }
            "desktop_screenshot" => {
                let (pid, number) = self.target(command, args)?;
                let result = self
                    .raw(
                        "get_window_state",
                        json!({"pid": pid, "window_id": number, "include_accessibility_tree": false}),
                    )
                    .map_err(|why| error(command, "DRIVER_FAILED", &why))?;
                if result.get("isError").and_then(Value::as_bool) == Some(true) {
                    return Err(refused(command, &result));
                }
                let mut content: Vec<Value> = result["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|block| block["type"] == "image")
                    .collect();
                let envelope = envelope_ok(command, json!({"window": format!("w-{number}")}));
                content.insert(0, json!({"type": "text", "text": envelope.to_string()}));
                Ok(json!({"content": content, "structuredContent": envelope, "isError": false}))
            }
            "desktop_get" | "desktop_is" => {
                let reference = text(args, "ref_id")
                    .ok_or_else(|| error(command, "INVALID_ARGS", "This needs ref_id."))?;
                let (_, _, _, element) = self.element(command, &reference)?;
                let element = element.unwrap_or(Value::Null);
                let property = text(args, "property").unwrap_or_else(|| "text".into());
                let value = match property.as_str() {
                    "text" | "value" => element
                        .get("value")
                        .cloned()
                        .or_else(|| element.get("label").cloned())
                        .unwrap_or(Value::Null),
                    "name" | "title" | "label" => {
                        element.get("label").cloned().unwrap_or(Value::Null)
                    }
                    "role" => element["role"].clone(),
                    "bounds" => ours("", &element)["bounds"].clone(),
                    "enabled" => json!(element["enabled"].as_bool().unwrap_or(true)),
                    "checked" | "selected" => json!(checked(&element)),
                    "visible" => json!(true),
                    other => element.get(other).cloned().unwrap_or(Value::Null),
                };
                Ok(result_ok(
                    command,
                    json!({"property": property, "value": value}),
                ))
            }
            "desktop_click" | "desktop_toggle" | "desktop_expand" | "desktop_collapse" => {
                self.on_ref(command, "click", args, json!({}))
            }
            "desktop_double_click" => self.on_ref(command, "double_click", args, json!({})),
            "desktop_right_click" => self.on_ref(command, "right_click", args, json!({})),
            "desktop_check" | "desktop_uncheck" => {
                let want = tool == "desktop_check";
                if let Some(reference) = text(args, "ref_id")
                    && let Ok((_, _, _, Some(element))) = self.element(command, &reference)
                    && element.get("value").is_some()
                    && checked(&element) == want
                {
                    return Ok(result_ok(command, json!({"changed": false})));
                }
                self.on_ref(command, "click", args, json!({}))
            }
            "desktop_set_value" | "desktop_select" => {
                let value = text_raw(args, "value");
                self.on_ref(command, "set_value", args, json!({"value": value}))
            }
            "desktop_clear" => self.on_ref(command, "set_value", args, json!({"value": ""})),
            "desktop_type" => {
                let typed = text_raw(args, "text");
                let body = typed.trim_end_matches(['\n', '\r']);
                let returns = typed[body.len()..]
                    .matches('\n')
                    .count()
                    .max(usize::from(typed.ends_with('\r') && !typed.ends_with('\n')));
                let (pid, number, token) = match text(args, "ref_id") {
                    Some(reference) => {
                        let (token, pid, number, _) = self.element(command, &reference)?;
                        (pid, number, Some(token))
                    }
                    None => {
                        let (pid, number) = self.target(command, args)?;
                        (pid, number, None)
                    }
                };
                let mut data = json!({});
                if !body.is_empty() {
                    let mut ask = json!({"pid": pid, "window_id": number, "text": body});
                    if let Some(token) = &token {
                        ask["element_token"] = json!(token);
                    }
                    data = self.data(command, "type_text", ask)?;
                }
                for _ in 0..returns {
                    data = self.data(
                        command,
                        "press_key",
                        json!({"pid": pid, "window_id": number, "key": "return"}),
                    )?;
                }
                Ok(result_ok(command, outcome(&data)))
            }
            "desktop_scroll" => {
                let direction = text(args, "direction").unwrap_or_else(|| "down".into());
                let amount = args.get("amount").and_then(Value::as_u64).unwrap_or(3);
                match text(args, "ref_id") {
                    Some(_) => self.on_ref(
                        command,
                        "scroll",
                        args,
                        json!({"direction": direction, "amount": amount}),
                    ),
                    None => {
                        let (pid, number) = self.target(command, args)?;
                        let data = self.data(
                            command,
                            "scroll",
                            json!({"pid": pid, "window_id": number, "direction": direction, "amount": amount}),
                        )?;
                        Ok(result_ok(command, outcome(&data)))
                    }
                }
            }
            "desktop_press" => {
                let combo = text(args, "combo").ok_or_else(|| {
                    error(
                        command,
                        "INVALID_ARGS",
                        "press needs combo (cmd+s, return, …).",
                    )
                })?;
                let (pid, number) = self.target(command, args)?;
                let keys = keys(&combo);
                let data = match keys.as_slice() {
                    [only] => self.data(
                        command,
                        "press_key",
                        json!({"pid": pid, "window_id": number, "key": only}),
                    )?,
                    _ => self.data(
                        command,
                        "hotkey",
                        json!({"pid": pid, "window_id": number, "keys": keys}),
                    )?,
                };
                Ok(result_ok(command, outcome(&data)))
            }
            "desktop_launch" => {
                let app = text(args, "app")
                    .ok_or_else(|| error(command, "INVALID_ARGS", "launch needs app."))?;
                let mut ask = if looks_like_bundle_id(&app) {
                    json!({"bundle_id": app})
                } else {
                    json!({"name": app})
                };
                if let Some(urls) = args.get("args").and_then(Value::as_array) {
                    let files: Vec<Value> = urls
                        .iter()
                        .filter(|arg| arg.as_str().is_some_and(|arg| !arg.starts_with('-')))
                        .cloned()
                        .collect();
                    if !files.is_empty() {
                        ask["urls"] = Value::Array(files);
                    }
                }
                let data = self.data(command, "launch_app", ask)?;
                let pid = data["pid"].as_i64().or_else(|| data["app"]["pid"].as_i64());
                let name = data["name"]
                    .as_str()
                    .or_else(|| data["app"]["name"].as_str())
                    .unwrap_or(&app)
                    .to_owned();
                Ok(result_ok(command, json!({"app": name, "pid": pid})))
            }
            "desktop_close_app" => {
                let app = text(args, "app")
                    .ok_or_else(|| error(command, "INVALID_ARGS", "close_app needs app."))?;
                let data = self.data(command, "list_apps", json!({}))?;
                let key = super::policy::app_key(&app);
                let pids: Vec<i32> = data["apps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|found| found["running"] == true)
                    .filter(|found| {
                        found["name"]
                            .as_str()
                            .is_some_and(|name| super::policy::app_key(name) == key)
                            || found["bundle_id"]
                                .as_str()
                                .is_some_and(|id| super::policy::app_key(id) == key)
                    })
                    .filter_map(|found| {
                        found["pid"]
                            .as_i64()
                            .and_then(|pid| i32::try_from(pid).ok())
                    })
                    .collect();
                if pids.is_empty() {
                    return Err(error(
                        command,
                        "APP_NOT_FOUND",
                        &format!("{app} isn't running."),
                    ));
                }
                let quit = pids
                    .iter()
                    .all(|pid| cfg!(test) || crate::assistant::actions::ax::terminate(*pid));
                Ok(result_ok(
                    command,
                    json!({"app": app, "asked_to_quit": quit}),
                ))
            }
            "desktop_focus_window" => {
                let (pid, number) = self.target(command, args)?;
                let data = self.data(
                    command,
                    "bring_to_front",
                    json!({"pid": pid, "window_id": number}),
                )?;
                Ok(result_ok(command, outcome(&data)))
            }
            "desktop_move_window" => {
                let (pid, number) = self.target(command, args)?;
                let at = (
                    args.get("x").and_then(Value::as_f64).unwrap_or_default(),
                    args.get("y").and_then(Value::as_f64).unwrap_or_default(),
                );
                if cfg!(test) {
                    return Ok(result_ok(command, json!({"window": format!("w-{number}")})));
                }
                crate::assistant::actions::ax::set_position(pid as i32, number, at).map_err(
                    |code| {
                        error(
                            command,
                            "ACTION_FAILED",
                            &format!("Accessibility error {code}"),
                        )
                    },
                )?;
                Ok(result_ok(
                    command,
                    json!({"window": format!("w-{number}"), "x": at.0, "y": at.1}),
                ))
            }
            "desktop_resize_window" => {
                let (pid, number) = self.target(command, args)?;
                let windows = self.windows(None)?;
                let bounds = windows
                    .iter()
                    .find(|window| window["window_id"].as_u64() == Some(u64::from(number)))
                    .map(|window| window["bounds"].clone())
                    .unwrap_or(Value::Null);
                let data = self.data(
                    command,
                    "set_window_frame",
                    json!({
                        "pid": pid,
                        "window_id": number,
                        "x": bounds["x"],
                        "y": bounds["y"],
                        "width": args.get("width"),
                        "height": args.get("height"),
                    }),
                )?;
                Ok(result_ok(command, outcome(&data)))
            }
            "desktop_minimize" | "desktop_restore" => {
                let (pid, number) = self.target(command, args)?;
                if cfg!(test) {
                    return Ok(result_ok(command, json!({"window": format!("w-{number}")})));
                }
                crate::assistant::actions::ax::set_minimized(
                    pid as i32,
                    number,
                    tool == "desktop_minimize",
                )
                .map_err(|code| {
                    error(
                        command,
                        "ACTION_FAILED",
                        &format!("Accessibility error {code}"),
                    )
                })?;
                Ok(result_ok(command, json!({"window": format!("w-{number}")})))
            }
            "desktop_wait" => self.wait(command, args),
            _ => Err(error(
                command,
                "UNSUPPORTED",
                "Cua Driver doesn't offer this; act through refs or hex_window_input.",
            )),
        }
    }

    fn wait(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let limit = Duration::from_millis(
            args.get("timeout")
                .or_else(|| args.get("ms"))
                .and_then(Value::as_u64)
                .unwrap_or(5000)
                .min(30_000),
        );
        let started = Instant::now();
        if let Some(event) = text(args, "event") {
            if event != "window-opened" {
                return Err(error(
                    command,
                    "UNSUPPORTED",
                    "Only event window-opened is waited for.",
                ));
            }
            let app = text(args, "app");
            let before: Vec<u64> = self
                .windows(app.as_deref())?
                .iter()
                .filter_map(|window| window["window_id"].as_u64())
                .collect();
            while started.elapsed() < limit {
                std::thread::sleep(Duration::from_millis(200));
                let now = self.windows(app.as_deref())?;
                if let Some(new) = now.iter().find(|window| {
                    window["window_id"]
                        .as_u64()
                        .is_some_and(|id| !before.contains(&id))
                }) {
                    return Ok(result_ok(
                        command,
                        json!({"window": {"id": format!("w-{}", new["window_id"]), "app_name": new["app_name"], "title": new["title"]}}),
                    ));
                }
            }
            return Err(error(command, "TIMEOUT", "No window opened in time."));
        }
        if let Some(wanted) = text(args, "text") {
            while started.elapsed() < limit {
                if let Ok(result) = self.snapshot(command, args, Some(wanted.clone()))
                    && result["structuredContent"]["data"]["tree"]
                        .as_array()
                        .is_some_and(|tree| !tree.is_empty())
                {
                    return Ok(result_ok(command, json!({"found": wanted})));
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            return Err(error(
                command,
                "TIMEOUT",
                &format!("“{wanted}” didn't show in time."),
            ));
        }
        if !cfg!(test) {
            std::thread::sleep(limit);
        }
        Ok(result_ok(
            command,
            json!({"waited_ms": limit.as_millis() as u64}),
        ))
    }
}

impl<I: Desktop> Desktop for Cua<I> {
    fn tools(&mut self) -> Result<Vec<Value>, String> {
        // Starts the driver: a missing or broken one fails here, and the
        // guard falls back to agent-desktop.
        self.inner.tools()?;
        Ok(tools())
    }

    fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
        Ok(match self.call_inner(tool, args) {
            Ok(result) | Err(result) => result,
        })
    }
}

/// The app in front, by pid.
fn frontmost() -> Option<i64> {
    if cfg!(test) {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        crate::dictation::context::frontmost_pid().map(i64::from)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn text(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A string argument as given (spaces and line ends kept).
fn text_raw(args: &Map<String, Value>, key: &str) -> String {
    match args.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    }
}

/// `(snapshot, row)` of a ref `@s0000002a:e11` (Cua's `s0000002a:11`).
pub fn token(reference: &str) -> Option<(String, u64)> {
    let bare = reference.trim().trim_start_matches('@');
    let (snapshot, element) = bare.split_once(':')?;
    let row = element.strip_prefix('e').unwrap_or(element).parse().ok()?;
    (!snapshot.is_empty()).then(|| (snapshot.to_owned(), row))
}

/// One of Cua's elements as the guard and the model read it.
fn ours(snapshot: &str, element: &Value) -> Value {
    let frame = &element["frame"];
    let role = element["role"].as_str().unwrap_or_default();
    let mut node = json!({
        "ref_id": format!("@{snapshot}:e{}", element["element_index"].as_u64().unwrap_or_default()),
        "role": role.strip_prefix("AX").unwrap_or(role).to_lowercase(),
        "depth": element["depth"],
    });
    let name = element["label"]
        .as_str()
        .or_else(|| element["title"].as_str())
        .unwrap_or_default();
    if !name.is_empty() {
        node["name"] = json!(name);
    }
    for key in ["value", "subrole", "enabled", "actions", "help"] {
        if let Some(value) = element.get(key).filter(|value| !value.is_null()) {
            node[key] = value.clone();
        }
    }
    if frame.is_object() {
        node["bounds"] = json!({
            "x": frame["x"], "y": frame["y"],
            "width": frame["w"].as_f64().or_else(|| frame["width"].as_f64()),
            "height": frame["h"].as_f64().or_else(|| frame["height"].as_f64()),
        });
    }
    node
}

/// A secure text field never gives up its value.
fn redact_secure(element: &mut Value) {
    let secure = ["role", "subrole"].iter().any(|key| {
        element[*key]
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains("secure"))
    });
    if secure && let Some(fields) = element.as_object_mut() {
        if fields.contains_key("value") {
            fields.insert("value".into(), json!({"redacted": true}));
        }
        fields.insert("states".into(), json!(["secure"]));
    }
}

/// The elements under row `root` (it included), by `parent_index`.
fn subtree(elements: &[Value], root: u64) -> Vec<Value> {
    let mut inside = vec![root];
    let mut kept = Vec::new();
    for element in elements {
        let index = element["element_index"].as_u64();
        let parent = element["parent_index"].as_u64();
        if index == Some(root) || parent.is_some_and(|parent| inside.contains(&parent)) {
            if let Some(index) = index {
                inside.push(index);
            }
            kept.push(element.clone());
        }
    }
    kept
}

/// A snapshot as indented lines: `[@ref] role "name" = value`.
fn outline(tree: &[Value]) -> String {
    tree.iter()
        .map(|node| {
            let depth = node["depth"].as_u64().unwrap_or(0).min(12) as usize;
            let mut line = format!(
                "{}- [{}] {}",
                "  ".repeat(depth),
                node["ref_id"].as_str().unwrap_or_default(),
                node["role"].as_str().unwrap_or_default()
            );
            if let Some(name) = node["name"].as_str() {
                line.push_str(&format!(" \"{name}\""));
            }
            match &node["value"] {
                Value::String(value) if !value.is_empty() => {
                    let short: String = value.chars().take(120).collect();
                    line.push_str(&format!(" = \"{short}\""));
                }
                Value::Number(value) => line.push_str(&format!(" = {value}")),
                Value::Object(_) => line.push_str(" = (secure)"),
                _ => {}
            }
            if node["enabled"] == false {
                line.push_str(" (disabled)");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn checked(element: &Value) -> bool {
    match &element["value"] {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => matches!(text.as_str(), "1" | "true" | "on"),
        _ => false,
    }
}

/// A combo (`cmd+shift+d`) as Cua's key names.
fn keys(combo: &str) -> Vec<String> {
    combo
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .map(|part| {
            match part.as_str() {
                "command" | "meta" | "⌘" | "super" => "cmd",
                "control" | "⌃" => "ctrl",
                "opt" | "alt" | "⌥" => "option",
                "⇧" => "shift",
                "enter" | "↩" => "return",
                "esc" => "escape",
                "backspace" | "⌫" => "delete",
                other => return other.to_owned(),
            }
            .to_owned()
        })
        .collect()
}

fn looks_like_bundle_id(app: &str) -> bool {
    !app.contains(' ') && !app.contains('/') && app.matches('.').count() >= 2
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// What an action's data says happened, kept short.
fn outcome(data: &Value) -> Value {
    let mut kept = json!({"disposition": {"delivery": "delivered"}});
    for key in ["effect", "route", "summary", "warnings", "window_change"] {
        if let Some(value) = data.get(key).filter(|value| !value.is_null()) {
            kept[key] = value.clone();
        }
    }
    kept
}

fn envelope_ok(command: &str, data: Value) -> Value {
    json!({"version": "cua-driver", "ok": true, "command": command, "data": data})
}

fn result_ok(command: &str, data: Value) -> Value {
    let envelope = envelope_ok(command, data);
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": false,
    })
}

fn error(command: &str, code: &str, message: &str) -> Value {
    let envelope = json!({
        "version": "cua-driver",
        "ok": false,
        "command": command,
        "error": {"code": code, "message": message},
    });
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": true,
    })
}

/// Cua's refusal as our error: its code (`stale_element_token` →
/// `STALE_REF`) and what it says, with its hint.
fn refused(command: &str, result: &Value) -> Value {
    let structured = &result["structuredContent"];
    let code = structured["refusal"]["code"]
        .as_str()
        .or_else(|| structured["code"].as_str())
        .or_else(|| structured["error"]["code"].as_str())
        .unwrap_or("ACTION_FAILED");
    let code = match code {
        "stale_element_token" | "unknown_element_token" => "STALE_REF".to_owned(),
        "window_id_not_found" | "window_not_found" => "WINDOW_NOT_FOUND".to_owned(),
        "minimized_or_hidden_window" => "WINDOW_MINIMIZED".to_owned(),
        other => other.to_uppercase(),
    };
    let message = text_of(result);
    let message = if message.is_empty() {
        structured.to_string()
    } else {
        // Cua names its own tools in hints; ours are the desktop_* ones.
        message
            .replace("get_window_state", "desktop_snapshot")
            .replace("element_token", "ref")
    };
    error(command, &code, &message)
}

/// The tools served (agent-desktop's names and arguments for what Cua
/// Driver does).
pub fn tools() -> Vec<Value> {
    let window = json!({"type": "string", "description": "The window's id (w-1234) from desktop_list_windows"});
    let app = json!({"type": "string", "description": "The app's name, when it has one window"});
    let reference =
        json!({"type": "string", "description": "A ref from a snapshot (@s00000001:e5)"});
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": {"type": "object", "properties": properties, "required": required},
        })
    };
    let on_ref = |name: &str, description: &str| {
        tool(
            name,
            description,
            json!({"ref_id": reference.clone()}),
            &["ref_id"],
        )
    };
    vec![
        tool(
            "desktop_status",
            "The driver and its permissions.",
            json!({}),
            &[],
        ),
        tool("desktop_version", "The driver's version.", json!({}), &[]),
        tool(
            "desktop_list_windows",
            "Windows on this Mac (id, app, title, pid, bounds, whether on screen).",
            json!({"app": app.clone()}),
            &[],
        ),
        tool(
            "desktop_list_apps",
            "Running apps (name, bundle id, pid).",
            json!({}),
            &[],
        ),
        tool(
            "desktop_snapshot",
            "A window's Accessibility elements with refs, as an indented outline. root drills \
             into one element's subtree; interactive_only keeps what acts.",
            json!({"window_id": window.clone(), "app": app.clone(), "root": reference.clone(),
                   "interactive_only": {"type": "boolean"}, "max_depth": {"type": "integer"}}),
            &[],
        ),
        tool(
            "desktop_find",
            "Elements of a window matching a name, text, or value (and role).",
            json!({"window_id": window.clone(), "app": app.clone(), "name": {"type": "string"},
                   "text": {"type": "string"}, "value": {"type": "string"}, "role": {"type": "string"}}),
            &[],
        ),
        tool(
            "desktop_screenshot",
            "A picture of one window (wherever it waits).",
            json!({"window_id": window.clone(), "app": app.clone()}),
            &[],
        ),
        tool(
            "desktop_get",
            "An element's property from its snapshot (text, value, name, role, bounds, enabled).",
            json!({"ref_id": reference.clone(), "property": {"type": "string"}}),
            &["ref_id"],
        ),
        tool(
            "desktop_is",
            "Whether an element is enabled, checked, or selected.",
            json!({"ref_id": reference.clone(), "property": {"type": "string"}}),
            &["ref_id", "property"],
        ),
        on_ref(
            "desktop_click",
            "Press an element (its Accessibility action), in the background.",
        ),
        on_ref(
            "desktop_double_click",
            "Open an element (double click), in the background.",
        ),
        on_ref(
            "desktop_right_click",
            "Show an element's menu, in the background.",
        ),
        on_ref("desktop_toggle", "Toggle a checkbox or switch."),
        on_ref("desktop_check", "Check a checkbox (nothing when it is)."),
        on_ref(
            "desktop_uncheck",
            "Uncheck a checkbox (nothing when it isn't).",
        ),
        on_ref("desktop_expand", "Expand a disclosure or row."),
        on_ref("desktop_collapse", "Collapse a disclosure or row."),
        on_ref("desktop_clear", "Empty a text field."),
        tool(
            "desktop_type",
            "Type text into an element (or the window's focused field); a line end at the end \
             presses Return.",
            json!({"ref_id": reference.clone(), "text": {"type": "string"}, "window_id": window.clone(), "app": app.clone()}),
            &["text"],
        ),
        tool(
            "desktop_set_value",
            "Set an element's value (a text field, slider, date, or a popup's item).",
            json!({"ref_id": reference.clone(), "value": {"type": "string"}}),
            &["ref_id", "value"],
        ),
        tool(
            "desktop_select",
            "Choose an item of a popup or list by its title.",
            json!({"ref_id": reference.clone(), "value": {"type": "string"}}),
            &["ref_id", "value"],
        ),
        tool(
            "desktop_scroll",
            "Scroll an element (or the window) up, down, left, or right by amount notches.",
            json!({"ref_id": reference.clone(), "window_id": window.clone(), "app": app.clone(),
                   "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                   "amount": {"type": "integer"}}),
            &["direction"],
        ),
        tool(
            "desktop_press",
            "Press keys in an app's window in the background (return, escape, tab, cmd+s, …).",
            json!({"combo": {"type": "string"}, "window_id": window.clone(), "app": app.clone()}),
            &["combo"],
        ),
        tool(
            "desktop_launch",
            "Start an app in the background (by name or bundle id); args may name files to open.",
            json!({"app": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}}}),
            &["app"],
        ),
        tool(
            "desktop_close_app",
            "Ask an app to quit (it may ask to save).",
            json!({"app": {"type": "string"}}),
            &["app"],
        ),
        tool(
            "desktop_focus_window",
            "Bring a window to the front (the person sees it).",
            json!({"window_id": window.clone(), "app": app.clone()}),
            &[],
        ),
        tool(
            "desktop_move_window",
            "Move a window's top-left corner to x, y (global points).",
            json!({"window_id": window.clone(), "app": app.clone(), "x": {"type": "number"}, "y": {"type": "number"}}),
            &["x", "y"],
        ),
        tool(
            "desktop_resize_window",
            "Resize a window.",
            json!({"window_id": window.clone(), "app": app.clone(), "width": {"type": "number"}, "height": {"type": "number"}}),
            &["width", "height"],
        ),
        tool(
            "desktop_minimize",
            "Minimize a window.",
            json!({"window_id": window.clone(), "app": app.clone()}),
            &[],
        ),
        tool(
            "desktop_restore",
            "Restore a minimized window.",
            json!({"window_id": window.clone(), "app": app.clone()}),
            &[],
        ),
        tool(
            "desktop_wait",
            "Wait for a window to open (event window-opened, app), for a text in a window, or ms.",
            json!({"ms": {"type": "integer"}, "event": {"type": "string"}, "app": app.clone(),
                   "text": {"type": "string"}, "window_id": window.clone(), "timeout": {"type": "integer"}}),
            &[],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Cua Driver, faked: answers like the recorded 0.49.1 and keeps calls.
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Vec<(String, Map<String, Value>)>>>);

    fn structured(value: Value) -> Value {
        json!({"content": [{"type": "text", "text": "ok"}], "structuredContent": value})
    }

    impl Desktop for Fake {
        fn tools(&mut self) -> Result<Vec<Value>, String> {
            Ok(Vec::new())
        }

        fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
            self.0.lock().unwrap().push((tool.to_owned(), args.clone()));
            Ok(match tool {
                "list_windows" => structured(json!({"windows": [
                    {"app_name": "TextEdit", "bounds": {"height": 439.0, "width": 673.0, "x": 195.0, "y": 92.0},
                     "is_on_screen": true, "layer": 0, "pid": 4921, "title": "pilot.txt", "window_id": 12796, "z_index": 0},
                ]})),
                "get_window_state" => structured(json!({
                "app_name": "TextEdit", "snapshot_id": "s00000001", "window_id": 12796, "window_title": "pilot.txt",
                "truncated": false,
                "elements": [
                    {"actions": ["AXRaise"], "depth": 0, "element_index": 0, "element_token": "s00000001:0",
                     "frame": {"h": 439.0, "w": 673.0, "x": 195.0, "y": 92.0}, "label": "pilot.txt", "role": "AXWindow"},
                    {"actions": ["AXShowMenu"], "depth": 1, "element_index": 1, "element_token": "s00000001:1",
                     "frame": {"h": 384.0, "w": 656.0, "x": 195.0, "y": 124.0}, "label": "seed", "parent_index": 0,
                     "role": "AXTextArea", "value": "seed line"},
                    {"actions": ["AXPress"], "depth": 1, "element_index": 2, "element_token": "s00000001:2",
                     "frame": {"h": 22.0, "w": 120.0, "x": 300.0, "y": 100.0}, "parent_index": 0,
                     "role": "AXTextField", "subrole": "AXSecureTextField", "value": "hunter2"},
                ]})),
                "click" if args.get("element_token") == Some(&json!("s00000001:9")) => json!({
                    "content": [{"type": "text", "text": "element_token is stale; call get_window_state again"}],
                    "isError": true,
                    "structuredContent": {"refusal": {"code": "stale_element_token", "message": "stale"}}}),
                _ => structured(json!({"effect": "confirmed", "route": "accessibility"})),
            })
        }
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn data(result: &Value) -> Value {
        result["structuredContent"]["data"].clone()
    }

    #[test]
    fn refs_map_to_cua_tokens_and_back() {
        assert_eq!(token("@s0000002a:e11"), Some(("s0000002a".into(), 11)));
        assert_eq!(token("s0000002a:11"), Some(("s0000002a".into(), 11)));
        assert_eq!(token("nonsense"), None);
        assert!(crate::assistant::guard::policy::snapshot_of("@s0000002a:e11").is_ok());
    }

    #[test]
    fn windows_snapshots_and_ref_actions_speak_agent_desktop() {
        let fake = Fake::default();
        let mut cua = Cua::new(fake.clone());
        let windows = cua.call("desktop_list_windows", &Map::new()).unwrap();
        assert_eq!(data(&windows)[0]["id"], "w-12796");
        assert_eq!(data(&windows)[0]["bounds"]["width"], 673.0);
        let snapshot = cua
            .call("desktop_snapshot", &args(json!({"window_id": "w-12796"})))
            .unwrap();
        let tree = data(&snapshot)["tree"].clone();
        assert_eq!(data(&snapshot)["snapshot_id"], "s00000001");
        assert_eq!(data(&snapshot)["window"]["id"], "w-12796");
        assert_eq!(tree[1]["ref_id"], "@s00000001:e1");
        assert_eq!(tree[1]["role"], "textarea");
        assert_eq!(tree[1]["bounds"]["x"], 195.0);
        // The password never shows.
        assert_eq!(tree[2]["value"], json!({"redacted": true}));
        let outline = snapshot["content"][0]["text"].as_str().unwrap();
        assert!(
            outline.contains("[@s00000001:e1] textarea \"seed\" = \"seed line\""),
            "{outline}"
        );
        assert!(!outline.contains("hunter2"));
        // The snapshot asked for no picture.
        let calls = fake.0.lock().unwrap().clone();
        let state = &calls
            .iter()
            .find(|(tool, _)| tool == "get_window_state")
            .unwrap()
            .1;
        assert_eq!(state["include_screenshot"], false);
        assert_eq!(state["pid"], 4921);
        // Typing with a Return at the end: text, then the key.
        let typed = cua
            .call(
                "desktop_type",
                &args(json!({"ref_id": "@s00000001:e1", "text": "hello\n"})),
            )
            .unwrap();
        assert_eq!(typed["structuredContent"]["ok"], true);
        let calls = fake.0.lock().unwrap().clone();
        let typed_call = &calls
            .iter()
            .find(|(tool, _)| tool == "type_text")
            .unwrap()
            .1;
        assert_eq!(typed_call["text"], "hello");
        assert_eq!(typed_call["element_token"], "s00000001:1");
        assert!(
            calls
                .iter()
                .any(|(tool, args)| tool == "press_key" && args["key"] == "return")
        );
    }

    #[test]
    fn refusals_become_error_envelopes() {
        let fake = Fake::default();
        let mut cua = Cua::new(fake);
        let _ = cua.call("desktop_snapshot", &args(json!({"window_id": "w-12796"})));
        let stale = cua
            .call("desktop_click", &args(json!({"ref_id": "@s00000001:e9"})))
            .unwrap();
        assert_eq!(stale["isError"], true);
        assert_eq!(stale["structuredContent"]["error"]["code"], "STALE_REF");
        assert!(
            stale["structuredContent"]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("desktop_snapshot")
        );
        let unknown = cua
            .call("desktop_click", &args(json!({"ref_id": "@sother:e1"})))
            .unwrap();
        assert_eq!(unknown["structuredContent"]["error"]["code"], "STALE_REF");
        let physical = cua.call("desktop_mouse_click", &Map::new()).unwrap();
        assert_eq!(
            physical["structuredContent"]["error"]["code"],
            "UNSUPPORTED"
        );
    }

    #[test]
    fn keys_and_launches_use_cua_s_names() {
        assert_eq!(keys("Cmd+Shift+D"), ["cmd", "shift", "d"]);
        assert_eq!(keys("enter"), ["return"]);
        assert_eq!(keys("ctrl+alt+backspace"), ["ctrl", "option", "delete"]);
        assert!(looks_like_bundle_id("com.apple.TextEdit"));
        assert!(!looks_like_bundle_id("TextEdit"));
        assert!(!looks_like_bundle_id("Visual Studio Code"));
        let fake = Fake::default();
        let mut cua = Cua::new(fake.clone());
        cua.call(
            "desktop_press",
            &args(json!({"combo": "cmd+s", "window_id": "w-12796"})),
        )
        .unwrap();
        let calls = fake.0.lock().unwrap().clone();
        let hotkey = &calls.iter().find(|(tool, _)| tool == "hotkey").unwrap().1;
        assert_eq!(hotkey["keys"], json!(["cmd", "s"]));
        assert_eq!(hotkey["pid"], 4921);
    }

    #[test]
    fn status_says_permissions_as_agent_desktop_did() {
        let mut cua = Cua::new(Fake::default());
        let status = cua.call("desktop_status", &Map::new()).unwrap();
        let grants = crate::assistant::backends::desktop::grants(&data(&status)).expect("grants");
        assert!(!grants.accessibility);
        let names: Vec<String> = tools()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        for name in &names {
            assert!(
                !matches!(
                    crate::assistant::guard::policy::kind(name, &Map::new()),
                    crate::assistant::guard::policy::Kind::Refused(_)
                ),
                "{name} would be refused"
            );
        }
        assert!(names.contains(&"desktop_snapshot".to_owned()));
    }
}

/// Live, by hand only (`cargo test --bin hex live_cua_driver -- --ignored`,
/// `HEX_CUA_DRIVER` the pinned `cua-driver`, `HEX_LIVE_FILE` a throwaway
/// text file): the adapter against the real driver and TextEdit in the
/// background, while another app keeps the front.
#[cfg(all(test, target_os = "macos"))]
mod live {
    use super::*;

    #[test]
    #[ignore = "drives TextEdit through Cua Driver on this Mac; run by hand"]
    fn live_cua_driver() {
        let driver = std::env::var("HEX_CUA_DRIVER").expect("HEX_CUA_DRIVER");
        let file = std::env::var("HEX_LIVE_FILE").expect("HEX_LIVE_FILE");
        let running = !std::process::Command::new("/usr/bin/pgrep")
            .args(["-x", "TextEdit"])
            .output()
            .unwrap()
            .stdout
            .is_empty();
        assert!(!running, "TextEdit may hold the person's documents");
        struct Quit;
        impl Drop for Quit {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/osascript")
                    .args([
                        "-e",
                        "tell application id \"com.apple.TextEdit\" to close every document saving no",
                        "-e",
                        "tell application id \"com.apple.TextEdit\" to quit saving no",
                    ])
                    .status();
            }
        }
        let _quit = Quit;
        let backend = crate::assistant::driver::Driver::Cua.backend();
        let mut cua = Cua::new(super::super::child::AgentDesktop::server(
            driver.into(),
            backend.mcp_args(None, "", false),
            backend.env(),
        ));
        assert!(!cua.tools().unwrap().is_empty());
        let front = crate::dictation::context::frontmost_pid();
        let call = |cua: &mut Cua<super::super::child::AgentDesktop>, tool: &str, args: Value| {
            cua.call(tool, args.as_object().unwrap()).unwrap()
        };
        let opened = call(
            &mut cua,
            "desktop_launch",
            json!({"app": "com.apple.TextEdit", "args": [file]}),
        );
        println!("launch: {}", opened["structuredContent"]);
        let mut window = None;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(250));
            let listed = call(&mut cua, "desktop_list_windows", json!({"app": "TextEdit"}));
            window = listed["structuredContent"]["data"]
                .as_array()
                .and_then(|windows| {
                    windows.iter().find(|window| {
                        window["title"]
                            .as_str()
                            .is_some_and(|title| !title.is_empty())
                    })
                })
                .and_then(|window| window["id"].as_str().map(str::to_owned));
            if window.is_some() {
                break;
            }
        }
        let window = window.expect("TextEdit's window");
        let snapshot = call(&mut cua, "desktop_snapshot", json!({"window_id": window}));
        println!(
            "{}",
            snapshot["content"][0]["text"].as_str().unwrap_or_default()
        );
        let area = snapshot["structuredContent"]["data"]["tree"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["role"] == "textarea")
            .map(|node| node["ref_id"].as_str().unwrap().to_owned())
            .expect("a text area");
        let typed = call(
            &mut cua,
            "desktop_type",
            json!({"ref_id": area, "text": "[cua] "}),
        );
        println!("type: {}", typed["structuredContent"]);
        let again = call(&mut cua, "desktop_snapshot", json!({"window_id": window}));
        let text = again["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        println!("{text}");
        assert_eq!(
            crate::dictation::context::frontmost_pid(),
            front,
            "the person's app kept the front"
        );
        assert!(text.contains("[cua]"), "{text}");
    }
}
