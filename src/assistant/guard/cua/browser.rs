//! The person's own browser window (Chrome, Edge, Brave), read and acted in
//! by its page through Cua Driver's typed browser tools: no pointer, no
//! keys, no copy of the page. The window is one the task holds, like any
//! other; the page's refs (`p3:7`) are Cua's.
//!
//! It needs the browser attached to Cua Driver (DevTools, which Chrome asks
//! the person about once): `agent-desk assistant driver browser --attach`.
//! Until it is, a call says so (`BROWSER_NOT_ATTACHED`), and the person's
//! tabs are read and used through `hex_browser_tab` (JavaScript from Apple
//! Events) instead: a capability, checked on each first use.

use serde_json::{Map, Value, json};

use super::{Cua, Desktop, error, outcome, result_ok, text, text_raw};

/// Cua Driver's refusals that mean the browser isn't attached yet.
const NOT_ATTACHED: &[&str] = &[
    "BROWSER_CONSENT_REQUIRED",
    "BROWSER_REQUIRES_SETUP",
    "BACKGROUND_UNAVAILABLE",
    "BROWSER_NOT_SUPPORTED",
    "UNSUPPORTED_BROWSER",
];

/// Most page refs a read returns.
const MAX_REFS: usize = 400;

impl<I: Desktop> Cua<I> {
    pub(super) fn browser(
        &mut self,
        command: &str,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Option<Result<Value, Value>> {
        Some(match tool {
            "desktop_browser" => self.page(command, args),
            "desktop_browser_act" => self.page_act(command, args),
            _ => return None,
        })
    }

    /// Binds the window's browser (attaching it in the background once)
    /// and reads the chosen tab.
    fn page(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let (pid, number) = self.target(command, args)?;
        let bind = json!({"pid": pid, "window_id": number});
        let bound = match self.data(command, "get_browser_state", bind.clone()) {
            Ok(bound) => bound,
            Err(refusal) if not_attached(&refusal) => {
                let prepared = self.data(
                    command,
                    "browser_prepare",
                    json!({"pid": pid, "window_id": number,
                           "strategy": {"kind": "existing_profile"},
                           "delivery_mode": "background"}),
                );
                match prepared {
                    Ok(_) => self.data(command, "get_browser_state", bind)?,
                    Err(_) => return Err(unattached(command)),
                }
            }
            Err(refusal) => return Err(refusal),
        };
        let target = bound["target_id"].as_str().unwrap_or_default().to_owned();
        let tabs = bound["tabs"].as_array().cloned().unwrap_or_default();
        let wanted = text(args, "tab");
        let found = tabs.iter().find(|tab| {
            wanted.as_deref().is_some_and(|wanted| {
                tab["tab_id"] == wanted
                    || ["title", "url"].iter().any(|key| {
                        tab[*key].as_str().is_some_and(|text| {
                            text.to_lowercase().contains(&wanted.to_lowercase())
                        })
                    })
            })
        });
        // A tab asked for by name that isn't there is an error, never the
        // tab the person is looking at (a navigate would replace it).
        if let Some(wanted) = &wanted
            && found.is_none()
            && !tabs.is_empty()
        {
            return Err(error(
                command,
                "TAB_NOT_FOUND",
                &format!(
                    "No tab matches “{wanted}” in that window; list its tabs with desktop_browser and pass one's tab_id."
                ),
            ));
        }
        let tab = found
            .or_else(|| {
                tabs.iter()
                    .find(|tab| tab["selected"] == true || tab["active"] == true)
            })
            .or(tabs.first())
            .and_then(|tab| tab["tab_id"].as_str())
            .map(str::to_owned)
            .or_else(|| bound["tab_id"].as_str().map(str::to_owned));
        let (Some(tab), false) = (tab, target.is_empty()) else {
            return Err(error(
                command,
                "BROWSER_NOT_ATTACHED",
                "Cua Driver found no tab in that browser window.",
            ));
        };
        self.browsers.insert(number, (target.clone(), tab.clone()));
        let mut ask = json!({"target_id": target, "tab_id": tab, "snapshot_format": "semantic_v2"});
        if let Some(query) = text(args, "query") {
            ask["query"] = json!(query);
        }
        let result = self
            .raw("get_browser_state", ask)
            .map_err(|why| error(command, "DRIVER_FAILED", &why))?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(super::refused(command, &result));
        }
        let mut refs = Vec::new();
        page_refs(&result["structuredContent"], &mut refs);
        let listed: Vec<Value> = tabs
            .iter()
            .map(|tab| json!({"tab": tab["tab_id"], "title": tab["title"], "url": tab["url"]}))
            .collect();
        let mut envelope = super::envelope_ok(
            command,
            json!({"window": format!("w-{number}"), "tab": tab, "tabs": listed, "refs": refs}),
        );
        envelope["data"]["page"] = json!(super::text_of(&result));
        Ok(json!({
            "content": [{"type": "text", "text": format!(
                "page of w-{number} (tab {tab}): act on a ref with desktop_browser_act\n{}",
                super::text_of(&result)
            )}],
            "structuredContent": envelope,
            "isError": false,
        }))
    }

    fn page_act(&mut self, command: &str, args: &Map<String, Value>) -> Result<Value, Value> {
        let (_, number) = self.target(command, args)?;
        let (target, tab) = self.browsers.get(&number).cloned().ok_or_else(|| {
            error(
                command,
                "INVALID_ARGS",
                "Read the page with desktop_browser first; its refs are what this acts on.",
            )
        })?;
        let mut ask = json!({"target_id": target, "tab_id": tab});
        let reference = text(args, "ref");
        let need_ref = || {
            reference.clone().ok_or_else(|| {
                error(
                    command,
                    "INVALID_ARGS",
                    "This action needs ref (from desktop_browser).",
                )
            })
        };
        let action = text(args, "action").unwrap_or_else(|| "click".into());
        let tool = match action.as_str() {
            "click" => {
                ask["ref"] = json!(need_ref()?);
                "browser_click"
            }
            "type" => {
                ask["ref"] = json!(need_ref()?);
                ask["text"] = json!(text_raw(args, "text"));
                ask["replace"] = json!(
                    args.get("replace")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                );
                "browser_type"
            }
            "select" => {
                ask["ref"] = json!(need_ref()?);
                ask["label"] = json!(text_raw(args, "label"));
                "browser_select"
            }
            "navigate" => {
                let url = text(args, "url")
                    .filter(|url| url.starts_with("http"))
                    .ok_or_else(|| {
                        error(command, "INVALID_ARGS", "navigate needs an http(s) url.")
                    })?;
                ask["url"] = json!(url);
                "browser_navigate"
            }
            "scroll" => {
                ask["action"] = json!("scroll");
                ask["delta_y"] =
                    json!(args.get("delta_y").and_then(Value::as_f64).unwrap_or(600.0));
                if let Some(reference) = &reference {
                    ask["ref"] = json!(reference);
                }
                "browser_pointer"
            }
            "dialog" => {
                ask["action"] = json!(text(args, "dialog").unwrap_or_else(|| "inspect".into()));
                if let Some(id) = text(args, "dialog_id") {
                    ask["dialog_id"] = json!(id);
                }
                "browser_dialog"
            }
            other => {
                return Err(error(
                    command,
                    "INVALID_ARGS",
                    &format!(
                        "“{other}” isn't an action: click, type, select, navigate, scroll, or dialog."
                    ),
                ));
            }
        };
        let data = self.data(command, tool, ask)?;
        let mut kept = outcome(&data);
        for key in ["dialog", "navigation", "url"] {
            if let Some(value) = data.get(key).filter(|value| !value.is_null()) {
                kept[key] = value.clone();
            }
        }
        Ok(result_ok(command, kept))
    }
}

fn not_attached(refusal: &Value) -> bool {
    refusal["structuredContent"]["error"]["code"]
        .as_str()
        .is_some_and(|code| NOT_ATTACHED.contains(&code))
}

fn unattached(command: &str) -> Value {
    error(
        command,
        "BROWSER_NOT_ATTACHED",
        "The person's browser isn't attached to Agent-Desk's driver (setup: `agent-desk assistant \
         driver browser --attach`, Chrome asks them once). Use hex_browser_tab for their tabs \
         instead.",
    )
}

/// Every page ref in a browser read, with what the guard needs to know
/// about it (its name, for consequential clicks).
fn page_refs(value: &Value, found: &mut Vec<Value>) {
    if found.len() >= MAX_REFS {
        return;
    }
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("ref").and_then(Value::as_str) {
                let name = ["name", "label", "text", "accessible_name"]
                    .iter()
                    .find_map(|key| fields.get(*key).and_then(Value::as_str))
                    .unwrap_or_default();
                found.push(json!({"ref_id": reference, "name": name, "role": fields.get("role")}));
            }
            for field in fields.values() {
                page_refs(field, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| page_refs(item, found)),
        _ => {}
    }
}

/// The tools served for the person's browser.
pub fn tools() -> Vec<Value> {
    let window = json!({"type": "string", "description": "The browser window's id (w-1234)"});
    vec![
        json!({
            "name": "desktop_browser",
            "description": "Read the page in a Chrome, Edge, or Brave window of yours (the \
                person's own browser, signed in): its tabs, and the page's elements with refs, \
                without the pointer or keys. tab picks a tab (its id, or words of its title or \
                URL); query narrows the read. When the browser isn't attached to Agent-Desk it \
                says so: then use hex_browser_tab.",
            "inputSchema": {"type": "object", "properties": {
                "window_id": window.clone(), "tab": {"type": "string"}, "query": {"type": "string"},
            }, "required": ["window_id"]},
        }),
        json!({
            "name": "desktop_browser_act",
            "description": "Act in the page desktop_browser read last in that window, by its \
                refs: click, type (text; replace to set the field), select (label of an option), \
                navigate (url), scroll (delta_y), dialog (inspect, accept, dismiss a page's \
                alert). Clicks that send, delete, buy, share, or publish follow the autonomy rules.",
            "inputSchema": {"type": "object", "properties": {
                "window_id": window,
                "action": {"type": "string", "enum": ["click", "type", "select", "navigate", "scroll", "dialog"]},
                "ref": {"type": "string"}, "text": {"type": "string"}, "replace": {"type": "boolean"},
                "label": {"type": "string"}, "url": {"type": "string"}, "delta_y": {"type": "number"},
                "dialog": {"type": "string"}, "dialog_id": {"type": "string"},
            }, "required": ["window_id", "action"]},
        }),
    ]
}
