//! Cua Driver's newer tools behind the guard, in its vocabulary: menus by
//! path (`invoke_menu`), drags posted to a window (`drag`), several steps
//! in one call (`run_steps`), checks of a window's state (`verify_state`),
//! a closer look at part of a window (`zoom`), regions found in its
//! picture (`parse_visual_regions`), the whole screen (`get_desktop_state`),
//! the clipboard, and a recording of the task's actions (`start_recording`).
//!
//! Every one names a window the guard has checked: `run_steps` gets the
//! batch's window and steps written only in the guard's terms (a ref of a
//! snapshot of that window, or a role and name), so no step can name
//! another window.

use serde_json::{Map, Value, json};

use super::{Cua, Desktop, MAX_ELEMENTS, error, keys, ours, outcome, outline, redact_secure};
use super::{refused, result_ok, text, text_raw};

/// What a step of `desktop_run_steps` may say: its action, its target (a
/// ref, or a role and name), and that action's own arguments.
const STEP_KEYS: &[&str] = &[
    "action",
    "ref_id",
    "role",
    "name",
    "nth",
    "text",
    "value",
    "combo",
    "direction",
    "amount",
    "from_x",
    "from_y",
    "to_x",
    "to_y",
    "expect",
    "wait_for",
    "timeout_ms",
];

/// The actions a step may take.
pub const STEP_ACTIONS: &[&str] = &[
    "click",
    "double_click",
    "right_click",
    "type",
    "set_value",
    "press",
    "scroll",
    "drag",
];

/// Most steps in one batch (Cua Driver's own limit).
const MAX_STEPS: usize = 64;

impl<I: Desktop> Cua<I> {
    /// The newer tools; `None` for any other.
    pub(super) fn more(
        &mut self,
        command: &str,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Option<Result<Value, Value>> {
        Some(match tool {
            "desktop_menu" => self.menu(command, args),
            "desktop_drag" => self.drag(command, args),
            "desktop_run_steps" => self.run_steps(command, args),
            "desktop_verify" => self.verify(command, args),
            "desktop_zoom" => self.zoom(command, args),
            "desktop_find_visual" => self.find_visual(command, args),
            "desktop_screen" => self.screen(command),
            "desktop_clipboard_get" => self
                .data(command, "clipboard_read", json!({"include_text": true}))
                .map(|data| {
                    result_ok(
                        command,
                        json!({"types": data["types"], "text": data["text"]}),
                    )
                }),
            "desktop_clipboard_set" => self.clipboard_set(command, args),
            "desktop_record_start" => {
                let dir = text(args, "dir").unwrap_or_default();
                self.data(
                    command,
                    "start_recording",
                    json!({"output_dir": dir, "include_accessibility_tree": false}),
                )
                .map(|data| result_ok(command, json!({"recording": data["recording"]})))
            }
            "desktop_record_stop" => self
                .data(command, "stop_recording", json!({}))
                .map(|data| result_ok(command, json!({"recording": data["recording"]}))),
            "desktop_browser" | "desktop_browser_act" => {
                return self.browser(command, tool, args);
            }
            _ => return None,
        })
    }

    /// A picture of the window registered with the driver, which its
    /// pixel tools (drag, zoom, regions) measure against.
    fn capture(&mut self, command: &str, pid: i64, number: u32) -> Result<Value, Value> {
        self.data(
            command,
            "get_window_state",
            json!({"pid": pid, "window_id": number, "include_screenshot": true,
                   "include_accessibility_tree": false}),
        )
    }

    fn menu(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let path: Vec<String> = args
            .get("path")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim).map(str::to_owned))
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if path.is_empty() || path.len() > 16 {
            return Err(error(
                command,
                "INVALID_ARGS",
                "menu needs path: the menu's titles from the menu bar down to the item, as the \
                 app shows them (its language), e.g. [\"File\", \"Export as PDF…\"].",
            ));
        }
        let (pid, number) = self.target(command, args)?;
        let data = self.data(
            command,
            "invoke_menu",
            json!({"pid": pid, "window_id": number, "path": path, "delivery_mode": "background"}),
        )?;
        let mut kept = outcome(&data);
        kept["path"] = json!(path);
        Ok(result_ok(command, kept))
    }

    fn drag(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let (pid, number, from, to) = match (text(args, "from"), text(args, "to")) {
            (Some(from), Some(to)) => {
                let (_, pid, number, start) = self.element(command, &from)?;
                let (_, other, end_number, end) = self.element(command, &to)?;
                if (other, end_number) != (pid, number) {
                    return Err(error(
                        command,
                        "INVALID_ARGS",
                        "from and to must be refs of the same window.",
                    ));
                }
                let center = |element: Option<Value>| {
                    element.as_ref().and_then(picture_center).ok_or_else(|| {
                        error(
                            command,
                            "INVALID_ARGS",
                            "That ref has no spot in the window's picture; drag by pixels \
                             (from_x, from_y, to_x, to_y) after a desktop_screenshot.",
                        )
                    })
                };
                (pid, number, center(start)?, center(end)?)
            }
            _ => {
                let (pid, number) = self.target(command, args)?;
                let number_of = |key: &str| {
                    args.get(key).and_then(Value::as_f64).ok_or_else(|| {
                        error(
                            command,
                            "INVALID_ARGS",
                            "drag needs from_x, from_y, to_x, to_y (pixels of the window's \
                             picture, as desktop_screenshot shows it).",
                        )
                    })
                };
                (
                    pid,
                    number,
                    (number_of("from_x")?, number_of("from_y")?),
                    (number_of("to_x")?, number_of("to_y")?),
                )
            }
        };
        self.capture(command, pid, number)?;
        let data = self.data(
            command,
            "drag",
            json!({"pid": pid, "window_id": number, "from_x": from.0, "from_y": from.1,
                   "to_x": to.0, "to_y": to.1, "delivery_mode": "background"}),
        )?;
        Ok(result_ok(command, outcome(&data)))
    }

    fn run_steps(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let steps = args
            .get("steps")
            .and_then(Value::as_array)
            .filter(|steps| !steps.is_empty() && steps.len() <= MAX_STEPS)
            .ok_or_else(|| {
                error(
                    command,
                    "INVALID_ARGS",
                    "run_steps needs steps: 1 to 64 actions, each {action, ref_id or role and name, …}.",
                )
            })?
            .clone();
        let (pid, number) = self.target(command, args)?;
        let mut cua_steps = Vec::new();
        let mut drags = false;
        for (index, step) in steps.iter().enumerate() {
            let step = step.as_object().cloned().unwrap_or_default();
            drags |= text(&step, "action").as_deref() == Some("drag");
            cua_steps.extend(self.step(command, index, &step, number)?);
        }
        if drags {
            self.capture(command, pid, number)?;
        }
        let mut ask = json!({"pid": pid, "window_id": number, "steps": cua_steps});
        if let Some(delay) = args.get("delay_ms").and_then(Value::as_u64) {
            ask["delay_ms"] = json!(delay.min(2000));
        }
        let observe = args.get("observe").and_then(Value::as_bool) != Some(false);
        if observe {
            ask["observe"] = json!({"tree_format": "elements", "max_elements": MAX_ELEMENTS,
                                    "since": null, "include_screenshot": false});
        }
        let data = self.data(command, "run_steps", ask)?;
        let ran: Vec<Value> = data["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|step| {
                json!({"index": step["index"], "tool": step["tool"], "ok": step["ok"],
                       "message": step["message"]})
            })
            .collect();
        let mut summary = vec![format!(
            "run_steps: {}/{} steps ran{}",
            data["executed"].as_u64().unwrap_or_default(),
            data["total"].as_u64().unwrap_or(cua_steps.len() as u64),
            match data["failed_step"].as_u64() {
                Some(failed) => format!(", step {failed} failed"),
                None => String::new(),
            }
        )];
        for step in &ran {
            summary.push(format!(
                "{}. {} {}: {}",
                step["index"].as_u64().unwrap_or_default() + 1,
                step["tool"].as_str().unwrap_or_default(),
                if step["ok"] == true { "ok" } else { "failed" },
                step["message"].as_str().unwrap_or_default()
            ));
        }
        let mut envelope = json!({
            "executed": data["executed"],
            "total": data["total"],
            "failed_step": data["failed_step"],
            "steps": ran,
        });
        let state = &data["observation"]["state"];
        if let Some(elements) = state["elements"].as_array().cloned()
            && let Some(snapshot) = state["snapshot_id"].as_str().map(str::to_owned)
        {
            let (tree, lines) = self.tree_of(&snapshot, pid, number, elements);
            envelope["snapshot_id"] = json!(snapshot);
            envelope["window"] =
                json!({"id": format!("w-{number}"), "title": state["window_title"]});
            envelope["app"] = state["app_name"].clone();
            envelope["tree"] = json!(tree);
            summary.push(format!(
                "snapshot {snapshot} of w-{number} after the steps:\n{lines}"
            ));
        }
        let failed = data["ok"] == false || !data["failed_step"].is_null();
        let mut result = result_ok(command, envelope);
        result["content"] = json!([{"type": "text", "text": summary.join("\n")}]);
        if failed {
            result["structuredContent"]["ok"] = json!(false);
            result["structuredContent"]["error"] = json!({
                "code": "STEP_FAILED",
                "message": summary.join("\n"),
            });
            result["isError"] = json!(true);
        }
        Ok(result)
    }

    /// One step in Cua Driver's terms (two when typed text ends in Return).
    fn step(
        &self,
        command: &str,
        index: usize,
        step: &Map<String, Value>,
        number: u32,
    ) -> Result<Vec<Value>, Value> {
        let bad = |why: String| {
            error(
                command,
                "INVALID_ARGS",
                &format!("step {}: {why}", index + 1),
            )
        };
        if let Some(key) = step.keys().find(|key| !STEP_KEYS.contains(&key.as_str())) {
            return Err(bad(format!(
                "“{key}” isn't a step argument (the batch's window_id names the window)."
            )));
        }
        let action = text(step, "action").unwrap_or_default();
        if !STEP_ACTIONS.contains(&action.as_str()) {
            return Err(bad(format!(
                "action must be one of {}.",
                STEP_ACTIONS.join(", ")
            )));
        }
        let mut target = Map::new();
        if let Some(reference) = text(step, "ref_id") {
            let (cua_token, _, of_window, _) = self.element(command, &reference)?;
            if of_window != number {
                return Err(bad(format!("{reference} is a ref of another window.")));
            }
            target.insert("element_token".into(), json!(cua_token));
        } else if let Some(name) = text(step, "name") {
            target.insert("name".into(), json!(name));
            if let Some(role) = text(step, "role") {
                target.insert("role".into(), json!(role));
            }
            if let Some(nth) = step.get("nth").and_then(Value::as_u64) {
                target.insert("nth".into(), json!(nth));
            }
        } else if let Some(role) = text(step, "role") {
            target.insert("role".into(), json!(role));
        }
        let needs_target = matches!(
            action.as_str(),
            "click" | "double_click" | "right_click" | "set_value"
        );
        if needs_target && target.is_empty() {
            return Err(bad(format!("{action} needs ref_id, or role and name.")));
        }
        let with = |tool: &str, extra: Value| -> Value {
            let mut args = target.clone();
            if let Some(extra) = extra.as_object() {
                args.extend(extra.clone());
            }
            json!({"tool": tool, "args": args})
        };
        let mut out = match action.as_str() {
            "click" | "double_click" | "right_click" => vec![with(&action, json!({}))],
            "set_value" => vec![with("set_value", json!({"value": text_raw(step, "value")}))],
            "type" => {
                let typed = text_raw(step, "text");
                let body = typed.trim_end_matches(['\n', '\r']);
                let returns = typed[body.len()..]
                    .matches('\n')
                    .count()
                    .max(usize::from(typed.ends_with('\r') && !typed.ends_with('\n')));
                let mut out = Vec::new();
                if !body.is_empty() {
                    out.push(with("type_text", json!({"text": body})));
                }
                for _ in 0..returns {
                    out.push(with("press_key", json!({"key": "return"})));
                }
                if out.is_empty() {
                    return Err(bad("type needs text.".into()));
                }
                out
            }
            "press" => {
                let combo = text(step, "combo").ok_or_else(|| bad("press needs combo.".into()))?;
                match keys(&combo).as_slice() {
                    [only] => vec![with("press_key", json!({"key": only}))],
                    many => vec![with("hotkey", json!({"keys": many}))],
                }
            }
            "scroll" => vec![with(
                "scroll",
                json!({"direction": text(step, "direction").unwrap_or_else(|| "down".into()),
                       "amount": step.get("amount").and_then(Value::as_u64).unwrap_or(3)}),
            )],
            _ => {
                let mut point = Map::new();
                for key in ["from_x", "from_y", "to_x", "to_y"] {
                    let value = step
                        .get(key)
                        .and_then(Value::as_f64)
                        .ok_or_else(|| bad(format!("drag needs {key} (window pixels).")))?;
                    point.insert(key.into(), json!(value));
                }
                vec![json!({"tool": "drag", "args": point})]
            }
        };
        // Checks look only in the batch's window.
        for key in ["expect", "wait_for"] {
            let Some(check) = step.get(key) else {
                continue;
            };
            let items: Vec<&Value> = match check {
                Value::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            if items
                .iter()
                .any(|item| item.get("app").is_some() || item.get("window").is_some())
            {
                return Err(bad(format!("{key} looks only in the batch's window.")));
            }
            if let Some(last) = out.last_mut().and_then(Value::as_object_mut) {
                last.insert(key.into(), check.clone());
            }
        }
        if let Some(timeout) = step.get("timeout_ms").and_then(Value::as_u64)
            && let Some(last) = out.last_mut().and_then(Value::as_object_mut)
        {
            last.insert("timeout_ms".into(), json!(timeout.min(10_000)));
        }
        Ok(out)
    }

    /// A snapshot's elements as the guard's tree and outline, its refs
    /// remembered.
    pub(super) fn tree_of(
        &mut self,
        snapshot: &str,
        pid: i64,
        number: u32,
        mut elements: Vec<Value>,
    ) -> (Vec<Value>, String) {
        for element in &mut elements {
            redact_secure(element);
        }
        self.remember(snapshot, pid, number, elements.clone());
        let tree: Vec<Value> = elements
            .iter()
            .map(|element| ours(snapshot, element))
            .collect();
        let lines = outline(&tree);
        (tree, lines)
    }

    fn verify(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let expect: Vec<Value> = args
            .get("expect")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(predicate).collect())
            .unwrap_or_default();
        if expect.is_empty() || expect.len() > 8 || expect.iter().any(Value::is_null) {
            return Err(error(
                command,
                "INVALID_ARGS",
                "verify needs expect: 1 to 8 checks, each {role, name, value, enabled, selected} \
                 of an element, or {window: {exists: true}}.",
            ));
        }
        let (pid, number) = self.target(command, args)?;
        let mut ask = json!({"pid": pid, "window_id": number, "expect": expect,
            "timeout_ms": args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(3000).min(10_000)});
        if let Some(samples) = args.get("stable_samples").and_then(Value::as_u64) {
            ask["stable_samples"] = json!(samples.clamp(1, 5));
        }
        let data = self.data(command, "verify_state", ask)?;
        let status = data["status"].as_str().unwrap_or("unknown").to_owned();
        let predicates: Vec<Value> = data["predicates"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|found| {
                let observed = found["observed_json"]
                    .as_str()
                    .and_then(|json| serde_json::from_str::<Value>(json).ok())
                    .map(|mut observed| {
                        redact_secure(&mut observed);
                        observed
                    })
                    .unwrap_or(Value::Null);
                json!({"index": found["index"], "status": found["status"],
                       "observed": observed, "unknown_reason": found["unknown_reason"]})
            })
            .collect();
        Ok(result_ok(
            command,
            json!({"status": status, "satisfied": status == "satisfied",
                   "predicates": predicates, "elapsed_ms": data["elapsed_ms"]}),
        ))
    }

    fn zoom(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let (pid, number) = self.target(command, args)?;
        let mut region = json!({"pid": pid, "window_id": number});
        for key in ["x1", "y1", "x2", "y2"] {
            let value = args.get(key).and_then(Value::as_f64).ok_or_else(|| {
                error(
                    command,
                    "INVALID_ARGS",
                    "zoom needs x1, y1 (top left) and x2, y2 (bottom right), in pixels of the \
                     window's picture.",
                )
            })?;
            region[key] = json!(value);
        }
        self.capture(command, pid, number)?;
        self.picture(
            command,
            "zoom",
            region,
            json!({"window": format!("w-{number}")}),
        )
    }

    /// A tool whose answer is a picture: the picture, after our envelope.
    fn picture(
        &mut self,
        command: &str,
        tool: &str,
        ask: Value,
        data: Value,
    ) -> Result<Value, Value> {
        let result = self
            .raw(tool, ask)
            .map_err(|why| error(command, "DRIVER_FAILED", &why))?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(refused(command, &result));
        }
        let mut data = data;
        for key in [
            "width",
            "height",
            "screen_width",
            "screen_height",
            "scale_factor",
        ] {
            if let Some(value) = result["structuredContent"].get(key) {
                data[key] = value.clone();
            }
        }
        let mut content: Vec<Value> = result["content"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|block| block["type"] == "image")
            .collect();
        let envelope = super::envelope_ok(command, data);
        content.insert(0, json!({"type": "text", "text": envelope.to_string()}));
        Ok(json!({"content": content, "structuredContent": envelope, "isError": false}))
    }

    fn find_visual(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let (pid, number) = self.target(command, args)?;
        let captured = self.capture(command, pid, number)?;
        let capture = captured["capture_id"].as_str().ok_or_else(|| {
            error(
                command,
                "ACTION_FAILED",
                "Cua Driver took no picture of that window.",
            )
        })?;
        let mut options = json!({});
        if let Some(kinds) = args.get("kinds").filter(|kinds| kinds.is_array()) {
            options["kinds"] = kinds.clone();
        }
        options["max_regions"] = json!(
            args.get("max_regions")
                .and_then(Value::as_u64)
                .unwrap_or(80)
                .min(300)
        );
        let parsed = self.data(
            command,
            "parse_visual_regions",
            json!({"capture_id": capture, "options": options}),
        );
        let data = match parsed {
            Ok(data) => data,
            Err(refusal)
                if refusal["structuredContent"]["error"]["code"] == "NOT_INSTALLED"
                    || refusal.to_string().contains("not installed") =>
            {
                return Err(error(
                    command,
                    "VISUAL_PARSER_MISSING",
                    "Cua Driver's visual-region parser isn't installed on this Mac. Read the \
                     window's text with hex_read_window, or look at it with hex_window_shot and \
                     act by its picture with hex_window_input.",
                ));
            }
            Err(refusal) => return Err(refusal),
        };
        Ok(result_ok(
            command,
            json!({"window": format!("w-{number}"), "regions": data["regions"],
                   "picture": {"width": captured["screenshot_width"],
                               "height": captured["screenshot_height"]}}),
        ))
    }

    fn screen(&mut self, command: &str) -> Result<Value, Value> {
        self.picture(
            command,
            "get_desktop_state",
            json!({"max_image_dimension": 1366}),
            json!({"screen": "main"}),
        )
    }

    fn clipboard_set(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let ask = if let Some(path) = text(args, "file_path") {
            json!({"file_path": path})
        } else if let Some(path) = text(args, "image_path") {
            json!({"image_path": path})
        } else if args.contains_key("text") {
            json!({"text": text_raw(args, "text")})
        } else {
            return Err(error(
                command,
                "INVALID_ARGS",
                "clipboard_set needs text, file_path, or image_path.",
            ));
        };
        let data = self.data(command, "clipboard_write", ask)?;
        Ok(result_ok(command, json!({"types": data["types"]})))
    }
}

/// A check in Cua Driver's terms: an element's (`{role, name, value,
/// enabled, selected}`, or `{element: …}` as Cua writes it) or the
/// window's (`{window: …}`); `Null` when it is neither.
fn predicate(item: &Value) -> Value {
    if item.get("element").is_some() || item.get("window").is_some() {
        return item.clone();
    }
    let mut selector = Map::new();
    if let Some(role) = item["role"].as_str() {
        selector.insert("role".into(), json!(role));
    }
    if let Some(name) = item["name"].as_str().or_else(|| item["label"].as_str()) {
        selector.insert("label_contains".into(), json!(name));
    }
    if selector.is_empty() {
        return Value::Null;
    }
    let mut element = json!({"selector": selector});
    if let Some(value) = item.get("value").filter(|value| !value.is_null()) {
        element["value_equals"] = json!(match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
    }
    for key in ["enabled", "selected"] {
        if let Some(flag) = item[key].as_bool() {
            element[key] = json!(flag);
        }
    }
    if element.get("value_equals").is_none()
        && element.get("enabled").is_none()
        && element.get("selected").is_none()
    {
        element["exists"] = json!(true);
    }
    json!({"element": element})
}

/// The center of an element in its window's picture (Cua's
/// `screenshot_frame`).
fn picture_center(element: &Value) -> Option<(f64, f64)> {
    let frame = &element["screenshot_frame"];
    let at = |key: &str, size: &str| Some(frame[key].as_f64()? + frame[size].as_f64()? / 2.0);
    Some((at("x", "w")?, at("y", "h")?))
}

/// The tools served for the newer Cua Driver tools.
pub fn tools() -> Vec<Value> {
    let window = json!({"type": "string", "description": "The window's id (w-1234)"});
    let app = json!({"type": "string", "description": "The app's name, when it has one window"});
    let pixel = json!({"type": "number"});
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": {"type": "object", "properties": properties, "required": required},
        })
    };
    let check = json!({"type": "object", "description":
        "{role, name, value, enabled, selected} of an element (name: part of its label), or {window: {exists: true}}"});
    vec![
        tool(
            "desktop_run_steps",
            "Several actions in one window in ONE call, in order, stopping at the first that \
             fails, then a fresh snapshot of the window (observe, on by default). Use it whenever \
             you know the next actions: fill fields, click, see the result. Each step: {action: \
             click | double_click | right_click | type | set_value | press | scroll | drag, ref_id \
             (from a snapshot of this window) or role and name (found fresh, waits up to 3 s), \
             text | value | combo | direction, amount | from_x, from_y, to_x, to_y, expect: \
             [{role, name, text, value, gone}] checked after it}. Each step is checked like its \
             own call; a step that would send, delete, pay, share, or publish needs the person's \
             allowance like any other.",
            json!({"window_id": window.clone(), "app": app.clone(),
                   "steps": {"type": "array", "items": {"type": "object"}, "maxItems": MAX_STEPS},
                   "observe": {"type": "boolean"}, "delay_ms": {"type": "integer"}}),
            &["steps"],
        ),
        tool(
            "desktop_run_script",
            "A JavaScript program that drives your windows in ONE call: loops, branches, and \
             retries between actions, and only its `return` value comes back. Use it when the \
             flow needs logic (a loop over rows, a branch on what the window shows); \
             desktop_run_steps for a fixed list. The body of an async function; its only API is \
             `desk`: desk.call(tool, args) runs any desktop_* tool you have (window_id defaults \
             to this call's), and desk.snapshot(), desk.click(ref), desk.type(ref, text), \
             desk.setValue(ref, value), desk.press(combo), desk.menu(path), desk.verify(expect), \
             desk.wait(ms); console.log works. Every call is checked by Agent-Desk like a call of \
             your own; a refused one throws an Error whose code says why.",
            json!({"script": {"type": "string"}, "window_id": window.clone(),
                   "timeout_ms": {"type": "integer"}, "max_calls": {"type": "integer"}}),
            &["script"],
        ),
        tool(
            "desktop_menu",
            "Run an app's menu item by its path from the menu bar, in the background, without \
             clicking: [\"File\", \"Export as PDF…\"]. Titles as the app shows them (its \
             language); a wrong title lists the items there.",
            json!({"window_id": window.clone(), "app": app.clone(),
                   "path": {"type": "array", "items": {"type": "string"}}}),
            &["path"],
        ),
        tool(
            "desktop_drag",
            "Drag in a window in the background (the person's pointer stays): from one ref to \
             another (from, to), or between pixels of the window's picture (from_x, from_y, \
             to_x, to_y). Sliders, canvases, selections; check the result afterwards.",
            json!({"window_id": window.clone(), "app": app.clone(),
                   "from": {"type": "string"}, "to": {"type": "string"},
                   "from_x": pixel.clone(), "from_y": pixel.clone(),
                   "to_x": pixel.clone(), "to_y": pixel.clone()}),
            &[],
        ),
        tool(
            "desktop_verify",
            "Check a window's state, waiting up to timeout_ms for it to settle: every check in \
             expect must hold (satisfied, unsatisfied, or unknown, which is never success). Use \
             it after a step that matters, before you say it is done.",
            json!({"window_id": window.clone(), "app": app.clone(),
                   "expect": {"type": "array", "items": check, "maxItems": 8},
                   "timeout_ms": {"type": "integer"}, "stable_samples": {"type": "integer"}}),
            &["expect"],
        ),
        tool(
            "desktop_zoom",
            "A closer picture of part of a window: x1, y1 (top left) to x2, y2 (bottom right) in \
             pixels of its picture (desktop_screenshot), to read small text or details.",
            json!({"window_id": window.clone(), "app": app.clone(), "x1": pixel.clone(),
                   "y1": pixel.clone(), "x2": pixel.clone(), "y2": pixel.clone()}),
            &["x1", "y1", "x2", "y2"],
        ),
        tool(
            "desktop_find_visual",
            "Text and icon regions found in a window's picture, with their boxes in its pixels: \
             for apps without an Accessibility tree (games, canvases, remote desktops).",
            json!({"window_id": window.clone(), "app": app.clone(),
                   "kinds": {"type": "array", "items": {"type": "string", "enum": ["text", "icon"]}},
                   "max_regions": {"type": "integer"}}),
            &[],
        ),
        tool(
            "desktop_screen",
            "A picture of the whole main screen, every window the person sees on it.",
            json!({}),
            &[],
        ),
        tool(
            "desktop_clipboard_get",
            "The clipboard's text and types (it is the person's too).",
            json!({}),
            &[],
        ),
        tool(
            "desktop_clipboard_set",
            "Put text, a file (file_path), or an image (image_path) on the clipboard, replacing \
             what the person copied.",
            json!({"text": {"type": "string"}, "file_path": {"type": "string"},
                   "image_path": {"type": "string"}}),
            &[],
        ),
    ]
}

/// The Cua Driver tool behind each of these (`driver::capabilities` reads
/// which Agent-Desk uses).
pub const USES: &[&str] = &[
    "invoke_menu",
    "drag",
    "run_steps",
    "verify_state",
    "zoom",
    "parse_visual_regions",
    "get_desktop_state",
    "clipboard_read",
    "clipboard_write",
    "start_recording",
    "stop_recording",
    "set_window_frame",
];

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Cua Driver 0.49.3, faked from what it answered on a TextEdit window.
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Vec<(String, Map<String, Value>)>>>);

    impl Fake {
        fn calls(&self, tool: &str) -> Vec<Map<String, Value>> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| name == tool)
                .map(|(_, args)| args.clone())
                .collect()
        }
    }

    fn structured(value: Value) -> Value {
        json!({"content": [{"type": "text", "text": "ok"}], "structuredContent": value})
    }

    fn element(index: u64, role: &str, label: &str) -> Value {
        json!({"element_index": index, "element_token": format!("s00000001:{index}"), "depth": 1,
               "role": role, "label": label, "actions": ["AXPress"],
               "frame": {"x": 200.0, "y": 130.0, "w": 100.0, "h": 20.0},
               "screenshot_frame": {"x": 10, "y": 76, "w": 200, "h": 40}})
    }

    impl Desktop for Fake {
        fn tools(&mut self) -> Result<Vec<Value>, String> {
            Ok(Vec::new())
        }

        fn call(&mut self, tool: &str, args: &Map<String, Value>) -> Result<Value, String> {
            self.0.lock().unwrap().push((tool.to_owned(), args.clone()));
            Ok(match tool {
                "list_windows" => structured(json!({"windows": [
                    {"app_name": "TextEdit", "bounds": {"height": 422.0, "width": 656.0, "x": 195.0, "y": 92.0},
                     "is_on_screen": true, "layer": 0, "pid": 74703, "title": "probe.txt",
                     "window_id": 16523, "z_index": 0}]})),
                "get_window_state" => structured(json!({
                    "app_name": "TextEdit", "snapshot_id": "s00000001", "window_id": 16523,
                    "window_title": "probe.txt", "capture_id": "capture_1",
                    "screenshot_width": 1312, "screenshot_height": 844,
                    "elements": [element(1, "AXTextArea", "seed line"), element(2, "AXButton", "OK")]})),
                "run_steps" => structured(json!({
                    "executed": 2, "total": 2, "failed_step": null, "ok": true,
                    "steps": [
                        {"index": 0, "tool": "type_text", "ok": true, "message": "Inserted 3 char(s)"},
                        {"index": 1, "tool": "click", "ok": true, "message": "pressed"}],
                    "observation": {"ok": true, "state": {
                        "app_name": "TextEdit", "snapshot_id": "s00000004", "window_title": "probe.txt",
                        "elements": [element(1, "AXTextArea", "Ada seed line")]}}})),
                "verify_state" => structured(json!({
                    "status": "satisfied", "elapsed_ms": 40, "samples": 2,
                    "predicates": [{"index": 0, "status": "satisfied", "unknown_reason": null,
                        "observed_json": "{\"role\":\"AXTextArea\",\"value\":\"Ada\"}"}]})),
                "parse_visual_regions" => json!({
                    "content": [{"type": "text", "text": "the optional cua-perception extension is not installed"}],
                    "isError": true,
                    "structuredContent": {"code": "not_installed", "retryable": false}}),
                "invoke_menu" if args["path"][0] == "Edit" => json!({
                    "content": [{"type": "text", "text": "invoke_menu: path segment 0 \"Edit\" was not found in the menu bar. Items there: \"Ablage\", \"Bearbeiten\""}],
                    "isError": true,
                    "structuredContent": {"refusal": {"code": "menu_path_unavailable"}, "status": "refused"}}),
                _ => structured(json!({"effect": "confirmed", "route": "accessibility"})),
            })
        }
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn a_batch_speaks_cua_s_steps_and_comes_back_with_refs() {
        let fake = Fake::default();
        let mut cua = Cua::new(fake.clone());
        cua.call("desktop_snapshot", &args(json!({"window_id": "w-16523"})))
            .unwrap();
        let ran = cua
            .call(
                "desktop_run_steps",
                &args(json!({"window_id": "w-16523", "steps": [
                    {"action": "type", "ref_id": "@s00000001:e1", "text": "Ada\n"},
                    {"action": "click", "role": "button", "name": "OK"},
                    {"action": "press", "combo": "cmd+s"},
                ]})),
            )
            .unwrap();
        let sent = &fake.calls("run_steps")[0];
        assert_eq!(sent["pid"], 74703);
        assert_eq!(sent["window_id"], 16523);
        assert_eq!(
            sent["steps"],
            json!([
                {"tool": "type_text", "args": {"element_token": "s00000001:1", "text": "Ada"}},
                {"tool": "press_key", "args": {"element_token": "s00000001:1", "key": "return"}},
                {"tool": "click", "args": {"role": "button", "name": "OK"}},
                {"tool": "hotkey", "args": {"keys": ["cmd", "s"]}},
            ])
        );
        assert_eq!(sent["observe"]["tree_format"], "elements");
        // The observation is a snapshot of ours: refs that act next.
        let data = &ran["structuredContent"]["data"];
        assert_eq!(data["snapshot_id"], "s00000004");
        assert_eq!(data["tree"][0]["ref_id"], "@s00000004:e1");
        assert!(cua.element("x", "@s00000004:e1").is_ok());
        assert!(
            ran["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("2/2 steps ran")
        );
        // A step names no window of its own.
        let refused = cua
            .call(
                "desktop_run_steps",
                &args(json!({"window_id": "w-16523", "steps": [{"action": "click", "name": "OK", "app": "Finder"}]})),
            )
            .unwrap();
        assert_eq!(
            refused["structuredContent"]["error"]["code"],
            "INVALID_ARGS"
        );
        assert_eq!(fake.calls("run_steps").len(), 1);
    }

    #[test]
    fn menus_checks_drags_and_regions_name_the_window() {
        let fake = Fake::default();
        let mut cua = Cua::new(fake.clone());
        let menu = cua
            .call(
                "desktop_menu",
                &args(json!({"window_id": "w-16523", "path": ["Format", "Schrift"]})),
            )
            .unwrap();
        assert_eq!(menu["structuredContent"]["ok"], true);
        let sent = &fake.calls("invoke_menu")[0];
        assert_eq!(sent["delivery_mode"], "background");
        assert_eq!(sent["pid"], 74703);
        let missing = cua
            .call(
                "desktop_menu",
                &args(json!({"window_id": "w-16523", "path": ["Edit", "Copy"]})),
            )
            .unwrap();
        assert_eq!(
            missing["structuredContent"]["error"]["code"],
            "MENU_PATH_UNAVAILABLE"
        );
        assert!(
            missing["structuredContent"]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Bearbeiten")
        );
        // A check in the guard's words becomes Cua's predicate.
        let verified = cua
            .call(
                "desktop_verify",
                &args(
                    json!({"window_id": "w-16523", "expect": [{"role": "textarea", "value": "Ada"},
                                                               {"name": "OK"}]}),
                ),
            )
            .unwrap();
        assert_eq!(verified["structuredContent"]["data"]["satisfied"], true);
        assert_eq!(
            verified["structuredContent"]["data"]["predicates"][0]["observed"]["value"],
            "Ada"
        );
        let expect = &fake.calls("verify_state")[0]["expect"];
        assert_eq!(
            expect,
            &json!([
                {"element": {"selector": {"role": "textarea"}, "value_equals": "Ada"}},
                {"element": {"selector": {"label_contains": "OK"}, "exists": true}},
            ])
        );
        // A drag between two refs goes between their spots in the picture,
        // after a picture is taken.
        cua.call("desktop_snapshot", &args(json!({"window_id": "w-16523"})))
            .unwrap();
        cua.call(
            "desktop_drag",
            &args(json!({"from": "@s00000001:e1", "to": "@s00000001:e2"})),
        )
        .unwrap();
        let drag = &fake.calls("drag")[0];
        assert_eq!(
            (drag["from_x"].clone(), drag["from_y"].clone()),
            (json!(110.0), json!(96.0))
        );
        assert_eq!(drag["delivery_mode"], "background");
        assert!(
            fake.calls("get_window_state")
                .iter()
                .any(|ask| ask["include_screenshot"] == true)
        );
        // Without Cua's visual parser the task is told its other ways.
        let regions = cua
            .call(
                "desktop_find_visual",
                &args(json!({"window_id": "w-16523"})),
            )
            .unwrap();
        assert_eq!(
            regions["structuredContent"]["error"]["code"],
            "VISUAL_PARSER_MISSING"
        );
        assert_eq!(
            fake.calls("parse_visual_regions")[0]["capture_id"],
            "capture_1"
        );
        // Moving a window on the screen is Cua's frame, read back.
        let moved = cua
            .call(
                "desktop_move_window",
                &args(json!({"window_id": "w-16523", "x": 300, "y": 120})),
            )
            .unwrap();
        assert_eq!(moved["structuredContent"]["data"]["verified"], true);
        assert_eq!(fake.calls("set_window_frame")[0]["width"], 656.0);
    }
}
