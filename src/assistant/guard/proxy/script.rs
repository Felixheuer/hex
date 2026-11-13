//! `desktop_run_script`: a task's JavaScript that drives its windows in one
//! call (loops, branches, retries between actions). It runs in Agent-Desk's
//! own QuickJS sandbox in the guard's process, never in the driver: its
//! only way out is `desk.call(tool, args)`, and every such call passes
//! [`Guard::run`] like a call of the task's own (its window, the lane,
//! consent, pause, blocked apps). No filesystem, network, process, or
//! timers; bounded time, memory, and calls.
//!
//! The script runs on a thread of its own; its desk calls come back here
//! over a channel and are answered one at a time, so the guard keeps its
//! single, synchronous view of the task.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, CaughtError, Context, Function, Promise, Runtime};
use serde_json::{Map, Value, json};

use super::super::child::{self, Desktop};
use super::{Guard, refusal, text};

/// Longest script, in bytes.
const MAX_SCRIPT: usize = 64 * 1024;
const MEMORY_LIMIT: usize = 32 << 20;
const STACK_LIMIT: usize = 1 << 20;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_CALLS: u64 = 100;
const MAX_CALLS: u64 = 300;
/// Most console lines kept.
const MAX_CONSOLE: usize = 200;

/// The tools a script may call: reads and actions in windows. Opening
/// apps, claiming windows, scripts of apps, and nested scripts are calls of
/// the task's own.
const SCRIPTABLE: &[&str] = &[
    "desktop_list_windows",
    "desktop_snapshot",
    "desktop_find",
    "desktop_get",
    "desktop_is",
    "desktop_click",
    "desktop_double_click",
    "desktop_right_click",
    "desktop_toggle",
    "desktop_check",
    "desktop_uncheck",
    "desktop_expand",
    "desktop_collapse",
    "desktop_clear",
    "desktop_type",
    "desktop_set_value",
    "desktop_select",
    "desktop_scroll",
    "desktop_press",
    "desktop_menu",
    "desktop_drag",
    "desktop_run_steps",
    "desktop_verify",
    "desktop_wait",
];

/// Calls that name their window: the script's `window_id` is theirs when
/// they name none.
const WINDOWED: &[&str] = &[
    "desktop_snapshot",
    "desktop_find",
    "desktop_type",
    "desktop_scroll",
    "desktop_press",
    "desktop_menu",
    "desktop_drag",
    "desktop_run_steps",
    "desktop_verify",
];

/// `desk` and `console`, the script's whole world.
const PRELUDE: &str = r#"
const desk = Object.freeze((() => {
  const call = (tool, args) => {
    const answer = JSON.parse(__deskCall(String(tool), JSON.stringify(args || {})));
    if (!answer.ok) {
      const error = new Error(answer.message || "refused");
      error.code = answer.code;
      error.tool = tool;
      throw error;
    }
    return answer.data;
  };
  return {
    call,
    snapshot: (args) => call("desktop_snapshot", args || {}),
    find: (args) => call("desktop_find", args || {}),
    click: (ref) => call("desktop_click", {ref_id: ref}),
    type: (ref, text) => text === undefined
      ? call("desktop_type", {text: String(ref)})
      : call("desktop_type", {ref_id: ref, text: String(text)}),
    setValue: (ref, value) => call("desktop_set_value", {ref_id: ref, value: String(value)}),
    press: (combo, ref) => call("desktop_press", ref ? {combo, ref_id: ref} : {combo}),
    menu: (path) => call("desktop_menu", {path}),
    steps: (steps, observe) => call("desktop_run_steps", {steps, observe: observe !== false}),
    verify: (expect, timeout_ms) =>
      call("desktop_verify", {expect: Array.isArray(expect) ? expect : [expect], timeout_ms}),
    wait: (ms) => call("desktop_wait", {ms}),
  };
})());
const console = Object.freeze((() => {
  const line = (...parts) => __deskLog(parts.map((part) =>
    typeof part === "string" ? part : JSON.stringify(part)).join(" "));
  return {log: line, info: line, warn: line, error: line, debug: line};
})());
"#;

/// From the script's thread to the guard.
enum FromScript {
    /// A desk call: its tool and arguments (JSON).
    Call(String, String),
    /// The script ended: its value (JSON) or why it failed, and its console.
    Done(Result<Value, String>, Vec<String>),
}

impl<D: Desktop> Guard<D> {
    /// Runs a task's script, every desk call of it checked by the guard.
    pub(super) fn run_script(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        let script = args
            .get("script")
            .and_then(Value::as_str)
            .filter(|script| !script.trim().is_empty())
            .ok_or_else(|| refusal(tool, "INVALID_ARGS", "run_script needs script."))?
            .to_owned();
        if script.len() > MAX_SCRIPT {
            return Err(refusal(
                tool,
                "INVALID_ARGS",
                "The script is longer than 64 KB; split the work.",
            ));
        }
        let limit = Duration::from_millis(
            args.get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(100, MAX_TIMEOUT_MS),
        );
        let max_calls = args
            .get("max_calls")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_CALLS)
            .clamp(1, MAX_CALLS);
        let window = text(args, "window_id");
        let deadline = Instant::now() + limit;
        let (to_guard, from_script) = mpsc::channel();
        let (to_script, from_guard) = mpsc::channel::<String>();
        let spawned = std::thread::Builder::new()
            .name("hex-run-script".into())
            .stack_size(8 << 20)
            .spawn(move || {
                let console = Rc::new(RefCell::new(Vec::new()));
                let outcome = evaluate(&script, deadline, &to_guard, from_guard, &console);
                let lines = console.borrow().clone();
                let _ = to_guard.send(FromScript::Done(outcome, lines));
            });
        if spawned.is_err() {
            return Err(refusal(tool, "ACTION_FAILED", "The script couldn't start."));
        }
        let mut calls = Vec::new();
        let (outcome, console) = loop {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(50))
                + Duration::from_secs(1);
            match from_script.recv_timeout(wait) {
                Ok(FromScript::Call(called, arguments)) => {
                    let answer = self.script_call(
                        &called,
                        &arguments,
                        window.as_deref(),
                        &mut calls,
                        max_calls,
                    );
                    let _ = to_script.send(answer.to_string());
                }
                Ok(FromScript::Done(outcome, console)) => break (outcome, console),
                Err(_) => {
                    break (
                        Err(format!("timed out after {} ms", limit.as_millis())),
                        Vec::new(),
                    );
                }
            }
        };
        let data = |value: Value| json!({"value": value, "console": console, "calls": calls.clone(), "calls_made": calls.len()});
        match outcome {
            Ok(value) => Ok(script_result(tool, true, data(value), None)),
            Err(why) => Err(script_result(
                tool,
                false,
                data(Value::Null),
                Some(format!("The script stopped: {why}")),
            )),
        }
    }

    /// One desk call of a script, checked like the task's own; its answer
    /// as the script reads it.
    fn script_call(
        &mut self,
        tool: &str,
        arguments: &str,
        window: Option<&str>,
        calls: &mut Vec<Value>,
        max_calls: u64,
    ) -> Value {
        let refused =
            |code: &str, message: &str| json!({"ok": false, "code": code, "message": message});
        let answer = if calls.len() as u64 >= max_calls {
            refused(
                "TOO_MANY_CALLS",
                &format!(
                    "The script made its {max_calls} calls; raise max_calls or split the work."
                ),
            )
        } else if !SCRIPTABLE.contains(&tool) {
            refused(
                "POLICY_DENIED",
                &format!(
                    "A script can't call {tool}; it reads and acts in your windows ({}).",
                    SCRIPTABLE.join(", ")
                ),
            )
        } else {
            let mut args: Map<String, Value> = serde_json::from_str(arguments).unwrap_or_default();
            let named = ["window_id", "app", "ref_id", "root", "from"]
                .iter()
                .any(|key| args.contains_key(*key));
            if let Some(window) = window
                && !named
                && WINDOWED.contains(&tool)
            {
                args.insert("window_id".into(), json!(window));
            }
            let result = match self.run(tool, &mut args) {
                Ok(result) | Err(result) => result,
            };
            answer_of(&result)
        };
        let mut logged = json!({"tool": tool, "ok": answer["ok"]});
        if let Some(code) = answer["code"].as_str() {
            logged["code"] = json!(code);
        }
        calls.push(logged);
        answer
    }
}

/// A guard result as a script reads it: `{ok, data}` or `{ok: false, code,
/// message}`.
fn answer_of(result: &Value) -> Value {
    let failed = result["isError"] == true;
    match child::envelope(result) {
        Some(envelope) if !failed && envelope["ok"] != false => {
            json!({"ok": true, "data": envelope.get("data").cloned().unwrap_or(Value::Null)})
        }
        Some(envelope) => json!({
            "ok": false,
            "code": envelope["error"]["code"].as_str().unwrap_or("FAILED"),
            "message": envelope["error"]["message"].as_str().unwrap_or_default(),
        }),
        None => {
            let text: Vec<&str> = result["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| block["text"].as_str())
                .collect();
            if failed {
                json!({"ok": false, "code": "FAILED", "message": text.join("\n")})
            } else {
                json!({"ok": true, "data": text.join("\n")})
            }
        }
    }
}

fn script_result(tool: &str, ok: bool, data: Value, error: Option<String>) -> Value {
    let mut envelope = json!({
        "version": "hex-guard",
        "ok": ok,
        "command": tool.trim_start_matches("desktop_").replace('_', "-"),
        "data": data,
    });
    if let Some(message) = &error {
        envelope["error"] = json!({"code": "SCRIPT_FAILED", "message": message});
    }
    let mut text = match &error {
        Some(message) => message.clone(),
        None => format!(
            "run_script: ok, {} call(s)\nreturn value: {}",
            envelope["data"]["calls_made"], envelope["data"]["value"]
        ),
    };
    if let Some(console) = envelope["data"]["console"].as_array()
        && !console.is_empty()
    {
        text.push_str("\nconsole:\n");
        let lines: Vec<&str> = console.iter().filter_map(Value::as_str).collect();
        text.push_str(&lines.join("\n"));
    }
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": envelope,
        "isError": !ok,
    })
}

/// Runs `script` (the body of an async function) to its end on this
/// thread: its value as JSON, or why it failed.
fn evaluate(
    script: &str,
    deadline: Instant,
    to_guard: &mpsc::Sender<FromScript>,
    from_guard: mpsc::Receiver<String>,
    console: &Rc<RefCell<Vec<String>>>,
) -> Result<Value, String> {
    let runtime = Runtime::new().map_err(|error| error.to_string())?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime).map_err(|error| error.to_string())?;
    let timed_out = || Instant::now() >= deadline;
    context.with(|ctx| {
        let sender = to_guard.clone();
        let call = Function::new(ctx.clone(), move |tool: String, args: String| -> String {
            if sender.send(FromScript::Call(tool, args)).is_err() {
                return r#"{"ok":false,"code":"STOPPED","message":"the guard stopped"}"#.into();
            }
            let wait = deadline.saturating_duration_since(Instant::now());
            from_guard.recv_timeout(wait).unwrap_or_else(|_| {
                r#"{"ok":false,"code":"TIMEOUT","message":"the script ran out of time"}"#.into()
            })
        })
        .map_err(|error| error.to_string())?;
        let lines = console.clone();
        let log = Function::new(ctx.clone(), move |line: String| {
            let mut lines = lines.borrow_mut();
            if lines.len() < MAX_CONSOLE {
                lines.push(line.chars().take(2000).collect());
            }
        })
        .map_err(|error| error.to_string())?;
        let globals = ctx.globals();
        globals
            .set("__deskCall", call)
            .and_then(|()| globals.set("__deskLog", log))
            .map_err(|error| error.to_string())?;
        ctx.eval::<(), _>(PRELUDE)
            .catch(&ctx)
            .map_err(|error| describe(error))?;
        let source = format!("(async function () {{\n\"use strict\";\n{script}\n}})()");
        let promise: Promise = ctx.eval(source).catch(&ctx).map_err(|error| {
            if timed_out() {
                "timed out".into()
            } else {
                describe(error)
            }
        })?;
        let value: rquickjs::Value = promise.finish().catch(&ctx).map_err(|error| {
            if timed_out() {
                "timed out".into()
            } else {
                describe(error)
            }
        })?;
        let json = ctx
            .json_stringify(value)
            .map_err(|error| error.to_string())?
            .and_then(|text| text.to_string().ok())
            .unwrap_or_else(|| "null".into());
        Ok(serde_json::from_str(&json).unwrap_or(Value::Null))
    })
}

/// A script's failure as the task reads it: the error, its code when a
/// desk call was refused, and where.
fn describe(error: CaughtError<'_>) -> String {
    match error {
        CaughtError::Exception(exception) => {
            let object = exception.as_object();
            let name = object
                .get::<_, String>("name")
                .unwrap_or_else(|_| "Error".into());
            let mut text = format!("{name}: {}", exception.message().unwrap_or_default());
            if let Ok(code) = object.get::<_, String>("code") {
                text.push_str(&format!(" (code {code})"));
            }
            if let Some(stack) = exception.stack() {
                for frame in stack
                    .lines()
                    .map(str::trim)
                    .filter(|frame| !frame.is_empty())
                {
                    text.push_str("\n  ");
                    text.push_str(frame);
                }
            }
            text
        }
        CaughtError::Value(value) if value.is_null() => "ran out of memory (or threw null)".into(),
        CaughtError::Value(value) => format!(
            "threw {}",
            value
                .as_string()
                .and_then(|text| text.to_string().ok())
                .unwrap_or_else(|| format!("a {}", value.type_name()))
        ),
        CaughtError::Error(rquickjs::Error::WouldBlock) => {
            "a promise never settled (desk calls are synchronous; nothing else can settle one)"
                .into()
        }
        CaughtError::Error(error) => error.to_string(),
    }
}
