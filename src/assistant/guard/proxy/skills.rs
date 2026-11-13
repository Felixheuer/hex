//! Skills: procedures that worked, per app and intent, done again next
//! time. `hex_skill_record` starts recording before a task does a
//! procedure and stops it when it worked; the guard keeps each action it
//! let through as a portable step (its target by role and name, never a
//! ref of one snapshot), and Cua Driver records the evidence beside it
//! (`start_recording`: each action's arguments and pictures before and
//! after). `hex_skills` lists them; `hex_skill_replay` runs one again in a
//! window of the task's through `desktop_run_steps` (every step checked by
//! the guard like its own call, consent included) and its menu items
//! through `desktop_menu`, then checks the recorded result with
//! `desktop_verify`.
//!
//! Cua Driver's own `replay_trajectory` replays recorded element tokens and
//! pixels, which go stale with the next snapshot (and Cua deprecates it in
//! favour of `run_steps`), so replays are the guard's.
//!
//! Kept in `<state>/skills/<app>/<intent>/skill.json`, the recording in
//! `recording/` beside it.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::super::child::{self, Desktop};
use super::super::policy::{self, SKILL_RECORD_TOOL, SKILL_REPLAY_TOOL, SKILLS_TOOL};
use super::{Guard, Resolved, refusal, same_app, text};

/// Most steps a skill keeps.
const MAX_STEPS: usize = 200;

/// A procedure being recorded.
pub struct Recording {
    app: String,
    intent: String,
    folder: PathBuf,
    steps: Vec<Value>,
    /// Cua Driver records the evidence too.
    evidence: bool,
}

/// A name as a folder name: lowercase letters, digits, and dashes.
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in policy::app_key(name).chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    if slug.is_empty() {
        "skill".into()
    } else {
        slug
    }
}

impl<D: Desktop> Guard<D> {
    /// `hex_skills`: the skills on this Mac (of `app`, when given).
    pub(super) fn skills_list(&mut self, args: &Map<String, Value>) -> Value {
        let app = text(args, "app");
        let skills: Vec<Value> = all(&self.skills_dir)
            .into_iter()
            .filter(|(_, skill)| {
                app.as_deref().is_none_or(|app| {
                    skill["app"]
                        .as_str()
                        .is_some_and(|found| same_app(found, app))
                })
            })
            .map(|(_, skill)| {
                json!({
                    "app": skill["app"], "intent": skill["intent"],
                    "steps": skill["steps"].as_array().map_or(0, Vec::len),
                    "checks": skill["expect"].as_array().map_or(0, Vec::len),
                    "runs": skill["runs"], "successes": skill["successes"],
                    "last_ok": skill["last_ok"],
                })
            })
            .collect();
        let envelope = json!({"version": "hex-guard", "ok": true, "command": "skills",
                              "data": {"skills": skills}});
        json!({"content": [{"type": "text", "text": envelope.to_string()}],
               "structuredContent": envelope, "isError": false})
    }

    /// `hex_skill_record`: start, stop, or cancel recording a procedure.
    pub(super) fn skill_record(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        match text(args, "action").as_deref().unwrap_or("start") {
            "start" => {
                let (Some(app), Some(intent)) = (text(args, "app"), text(args, "intent")) else {
                    return Err(refusal(
                        tool,
                        "INVALID_ARGS",
                        "Name the app and the intent (what the procedure does, in a few words).",
                    ));
                };
                if let Some(recording) = &self.recording {
                    return Err(refusal(
                        tool,
                        "INVALID_ARGS",
                        &format!(
                            "Already recording “{}” in {}; stop it first.",
                            recording.intent, recording.app
                        ),
                    ));
                }
                let folder = self.skills_dir.join(slug(&app)).join(slug(&intent));
                let partial = folder.join("recording.partial");
                let _ = std::fs::remove_dir_all(&partial);
                let evidence = std::fs::create_dir_all(&partial).is_ok()
                    && self
                        .forward(
                            "desktop_record_start",
                            &args_of(json!({"dir": partial.display().to_string()})),
                        )
                        .is_ok_and(|result| result["isError"] != true);
                self.recording = Some(Recording {
                    app: app.clone(),
                    intent: intent.clone(),
                    folder,
                    steps: Vec::new(),
                    evidence,
                });
                Ok(ok(
                    tool,
                    json!({"recording": true, "app": app, "intent": intent}),
                    "Recording. Do the procedure now; when it worked, call hex_skill_record with \
                     action stop and succeeded true (and expect: the checks that show it worked).",
                ))
            }
            "stop" | "cancel" => {
                let Some(recording) = self.recording.take() else {
                    return Err(refusal(tool, "INVALID_ARGS", "Nothing is being recorded."));
                };
                if recording.evidence {
                    let _ = self.forward("desktop_record_stop", &Map::new());
                }
                let partial = recording.folder.join("recording.partial");
                let succeeded = text(args, "action").as_deref() == Some("stop")
                    && args.get("succeeded").and_then(Value::as_bool) == Some(true);
                if !succeeded || recording.steps.is_empty() {
                    let _ = std::fs::remove_dir_all(&partial);
                    return Ok(ok(
                        tool,
                        json!({"saved": false, "steps": recording.steps.len()}),
                        "Not saved: only a procedure that worked (and did something) is kept.",
                    ));
                }
                let expect: Vec<Value> = args
                    .get("expect")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .take(8)
                    .collect();
                let now = crate::desktop::events::now_ms();
                let skill = json!({
                    "app": recording.app, "intent": recording.intent,
                    "steps": recording.steps, "expect": expect,
                    "created_ms": now, "updated_ms": now,
                    "runs": 0, "successes": 0, "last_ok": null,
                });
                save(&recording.folder, &skill).map_err(|why| {
                    refusal(
                        tool,
                        "ACTION_FAILED",
                        &format!("Couldn't keep the skill: {why}"),
                    )
                })?;
                let kept = recording.folder.join("recording");
                let _ = std::fs::remove_dir_all(&kept);
                let _ = std::fs::rename(&partial, &kept);
                Ok(ok(
                    tool,
                    json!({"saved": true, "app": skill["app"], "intent": skill["intent"],
                           "steps": skill["steps"].as_array().map_or(0, Vec::len)}),
                    "Saved: next time hex_skill_replay does it in one call.",
                ))
            }
            other => Err(refusal(
                tool,
                "INVALID_ARGS",
                &format!("“{other}” isn't an action: start, stop, or cancel."),
            )),
        }
    }

    /// While recording, keeps an action the guard let through (and the
    /// driver did) as portable steps.
    pub(super) fn note_step(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
        result: &Value,
    ) {
        let Some(recording) = &self.recording else {
            return;
        };
        if result["isError"] == true
            || !resolved
                .iter()
                .any(|window| same_app(&window.app, &recording.app))
        {
            return;
        }
        let mut steps = Vec::new();
        match tool {
            "desktop_run_steps" => {
                for step in args
                    .get("steps")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(step) = step.as_object() {
                        let action = text(step, "action").unwrap_or_default();
                        steps.extend(self.portable(&action, step));
                    }
                }
            }
            "desktop_menu" => steps.push(json!({"action": "menu", "path": args.get("path")})),
            other => {
                let action = match other.trim_start_matches("desktop_") {
                    "toggle" | "check" | "uncheck" | "expand" | "collapse" => "click",
                    "select" => "set_value",
                    action => action,
                };
                steps.extend(self.portable(action, args));
            }
        }
        if let Some(recording) = &mut self.recording {
            for step in steps {
                if recording.steps.len() < MAX_STEPS {
                    recording.steps.push(step);
                }
            }
        }
    }

    /// One action as a step that finds its target fresh (role and name),
    /// or `None` when it can't be told again (a ref without a name).
    fn portable(&self, action: &str, args: &Map<String, Value>) -> Option<Value> {
        if !super::super::cua::STEP_ACTIONS.contains(&action) {
            return None;
        }
        let mut step = Map::new();
        step.insert("action".into(), json!(action));
        let element = text(args, "ref_id")
            .and_then(|reference| self.learned.elements.lock().get(&reference).cloned());
        let mut secure = false;
        match element {
            Some(element) => {
                // A field named by its own text (TextEdit's) is found by its
                // role: the text is different next time.
                let named = !element.label.is_empty() && element.label != element.value;
                if !named {
                    // Found again only by its role: fine for the one field.
                    if !matches!(action, "type" | "set_value" | "scroll") {
                        return None;
                    }
                } else {
                    step.insert("name".into(), json!(element.label));
                }
                if !element.role.is_empty() {
                    step.insert("role".into(), json!(element.role));
                }
                secure = element.secure;
            }
            None => {
                for key in ["role", "name", "nth"] {
                    if let Some(value) = args.get(key) {
                        step.insert(key.into(), value.clone());
                    }
                }
                if text(args, "ref_id").is_some() {
                    return None;
                }
            }
        }
        for key in [
            "combo",
            "direction",
            "amount",
            "from_x",
            "from_y",
            "to_x",
            "to_y",
        ] {
            if let Some(value) = args.get(key) {
                step.insert(key.into(), value.clone());
            }
        }
        for key in ["text", "value"] {
            if let Some(value) = args.get(key) {
                if secure {
                    // A password is never kept: the replay asks for it.
                    step.insert("secret".into(), json!(true));
                } else {
                    step.insert(key.into(), value.clone());
                }
            }
        }
        Some(Value::Object(step))
    }

    /// `hex_skill_replay`: runs a skill again in one of the task's windows
    /// and checks its result.
    pub(super) fn skill_replay(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Result<Value, Value> {
        let intent = text(args, "intent").ok_or_else(|| {
            refusal(
                tool,
                "INVALID_ARGS",
                "Name the skill's intent (hex_skills lists them).",
            )
        })?;
        let app = text(args, "app");
        let window = text(args, "window_id").ok_or_else(|| {
            refusal(
                tool,
                "INVALID_ARGS",
                "Name your window to run it in with window_id.",
            )
        })?;
        let wanted = slug(&intent);
        let (folder, mut skill) = all(&self.skills_dir)
            .into_iter()
            .filter(|(_, skill)| {
                app.as_deref().is_none_or(|app| {
                    skill["app"]
                        .as_str()
                        .is_some_and(|found| same_app(found, app))
                })
            })
            .find(|(_, skill)| {
                skill["intent"]
                    .as_str()
                    .is_some_and(|found| slug(found) == wanted)
            })
            .ok_or_else(|| {
                refusal(
                    tool,
                    "SKILL_NOT_FOUND",
                    "There's no such skill; hex_skills lists them. Do it step by step instead.",
                )
            })?;
        let mut values = args
            .get("values")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter();
        // Typed text and values in order, the person's own where given.
        let mut steps: Vec<Value> = skill["steps"].as_array().cloned().unwrap_or_default();
        for step in &mut steps {
            let Some(fields) = step.as_object_mut() else {
                continue;
            };
            let key = match fields.get("action").and_then(Value::as_str) {
                Some("type") => "text",
                Some("set_value") => "value",
                _ => continue,
            };
            match values.next() {
                Some(value) => {
                    fields.insert(key.into(), value);
                }
                None if fields.remove("secret").is_some() => {
                    return Err(refusal(
                        tool,
                        "INVALID_ARGS",
                        "This skill types a secret, which is never kept: pass it in values.",
                    ));
                }
                None => {}
            }
            fields.remove("secret");
        }
        // Menu items are their own calls; the steps between run as batches.
        let mut segments: Vec<Value> = Vec::new();
        let mut batch: Vec<Value> = Vec::new();
        for step in steps {
            if step["action"] == "menu" {
                if !batch.is_empty() {
                    segments.push(json!({"steps": std::mem::take(&mut batch)}));
                }
                segments.push(json!({"menu": step["path"]}));
            } else {
                batch.push(step);
            }
        }
        if !batch.is_empty() {
            segments.push(json!({"steps": batch}));
        }
        let total = segments.len();
        let mut ran = Vec::new();
        let mut failed = None;
        for (index, segment) in segments.into_iter().enumerate() {
            let last = index + 1 == total;
            let (called, mut call) = match segment.get("menu") {
                Some(path) => (
                    "desktop_menu",
                    args_of(json!({"window_id": window, "path": path})),
                ),
                None => (
                    "desktop_run_steps",
                    args_of(
                        json!({"window_id": window, "steps": segment["steps"], "observe": last}),
                    ),
                ),
            };
            let result = match self.run(called, &mut call) {
                Ok(result) | Err(result) => result,
            };
            let ok = result["isError"] != true;
            ran.push(json!({"call": called, "ok": ok, "result": child::envelope(&result)}));
            if !ok {
                failed = Some(index);
                break;
            }
        }
        let expect = skill["expect"].as_array().cloned().unwrap_or_default();
        let verified = if failed.is_none() && !expect.is_empty() {
            let mut call = args_of(json!({"window_id": window, "expect": expect}));
            let result = match self.run("desktop_verify", &mut call) {
                Ok(result) | Err(result) => result,
            };
            Some(
                child::data(&result)
                    .and_then(|data| data["status"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".into()),
            )
        } else {
            None
        };
        let worked = failed.is_none()
            && verified
                .as_deref()
                .is_none_or(|status| status == "satisfied");
        skill["runs"] = json!(skill["runs"].as_u64().unwrap_or(0) + 1);
        if worked {
            skill["successes"] = json!(skill["successes"].as_u64().unwrap_or(0) + 1);
        }
        skill["last_ok"] = json!(worked);
        skill["updated_ms"] = json!(crate::desktop::events::now_ms());
        let _ = save(&folder, &skill);
        let data = json!({
            "skill": {"app": skill["app"], "intent": skill["intent"]},
            "worked": worked, "failed_at": failed, "verified": verified, "ran": ran,
        });
        let said = if worked && verified.is_some() {
            "The skill ran and its checks hold."
        } else if worked {
            "The skill ran; it has no recorded checks, so look at the window before you report it done."
        } else {
            "The skill didn't work this time (the app may have changed): look at the window and \
             do the rest step by step; record it again once it works."
        };
        if worked {
            Ok(ok(tool, data, said))
        } else {
            let mut result = ok(tool, data, said);
            result["isError"] = json!(true);
            result["structuredContent"]["ok"] = json!(false);
            result["structuredContent"]["error"] = json!({"code": "SKILL_FAILED", "message": said});
            Err(result)
        }
    }
}

fn args_of(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn ok(tool: &str, data: Value, said: &str) -> Value {
    let envelope = json!({
        "version": "hex-guard",
        "ok": true,
        "command": tool.trim_start_matches("hex_").replace('_', "-"),
        "data": data,
    });
    json!({
        "content": [{"type": "text", "text": format!("{said}\n{envelope}")}],
        "structuredContent": envelope,
        "isError": false,
    })
}

fn save(folder: &Path, skill: &Value) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let partial = folder.join(".skill.json.partial");
    std::fs::write(
        &partial,
        serde_json::to_vec_pretty(skill).unwrap_or_default(),
    )?;
    std::fs::rename(&partial, folder.join("skill.json"))
}

/// Every skill under `dir`, with its folder.
fn all(dir: &Path) -> Vec<(PathBuf, Value)> {
    let mut found = Vec::new();
    let Ok(apps) = std::fs::read_dir(dir) else {
        return found;
    };
    for app in apps.filter_map(Result::ok) {
        let Ok(intents) = std::fs::read_dir(app.path()) else {
            continue;
        };
        for intent in intents.filter_map(Result::ok) {
            let folder = intent.path();
            if let Some(skill) = std::fs::read_to_string(folder.join("skill.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            {
                found.push((folder, skill));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The skill tools' schemas.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": SKILLS_TOOL,
            "description": "Agent-Desk: the procedures recorded on this Mac (skills), per app and \
                intent, with how often they worked. Look here before a procedure of several \
                steps in an app; replay a matching one with hex_skill_replay.",
            "inputSchema": {"type": "object", "properties": {
                "app": {"type": "string", "description": "Only this app's skills"},
            }},
        }),
        json!({
            "name": SKILL_RECORD_TOOL,
            "description": "Agent-Desk: record a procedure as you do it, to replay it next time. \
                action start (with app and intent, a few words saying what it does) before the \
                first step; action stop with succeeded true once it worked, and expect: the \
                checks that show it (as desktop_verify takes them); cancel to drop it.",
            "inputSchema": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["start", "stop", "cancel"]},
                "app": {"type": "string"}, "intent": {"type": "string"},
                "succeeded": {"type": "boolean"},
                "expect": {"type": "array", "items": {"type": "object"}, "maxItems": 8},
            }, "required": ["action"]},
        }),
        json!({
            "name": SKILL_REPLAY_TOOL,
            "description": "Agent-Desk: do a recorded procedure (hex_skills) again in one of your \
                windows, in one call, each step checked like your own, then check its recorded \
                result. values replaces the text it types, in order. If it fails, do the rest \
                step by step.",
            "inputSchema": {"type": "object", "properties": {
                "intent": {"type": "string"}, "app": {"type": "string"},
                "window_id": {"type": "string"},
                "values": {"type": "array", "items": {"type": "string"}},
            }, "required": ["intent", "window_id"]},
        }),
    ]
}
