//! Consequential actions and secure fields. Before a click, a key combo, or
//! text ending in Return, the guard asks what it would do (`autonomy`), the
//! same way in every app, without lists of apps:
//!
//! - a click: the element's own label as the snapshot showed it ("Send",
//!   "Move to Trash", "Jetzt kaufen", a price);
//! - a key combo: the target app's menu item with that key equivalent, read
//!   from its menu bar over Accessibility (⇧⌘D is "Send" in one app, ⌘⌫
//!   "Move to Trash" in another), classified by its title;
//! - Return (pressed, or ending typed text): in a one-line field or outside
//!   a field, the window's default button; in a field, the nearest
//!   send-like control by its label (its container's buttons, never the
//!   whole window's: Return in a message body is a new line).
//!
//! What can't be told is treated like any other action under the mode.
//! Unless the person's mode and what they authorized for this task allow
//! it, a consequential action is refused with `NEEDS_CONFIRMATION`. What a
//! task reads on screen never authorizes anything.
//!
//! Secure text fields (passwords) never give up their value: snapshot and
//! find results have it redacted, and reading it is refused.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::super::screen::{MenuKey, ReturnTarget, input_role};
use super::{Guard, Look, Resolved, refusal, same_app, text};
use crate::assistant::autonomy::{self, Autonomy, Category};

/// The guard's refusal for an action the person has to allow first: its
/// envelope, with the gate's one `NEEDS_CONFIRMATION` error.
pub(super) fn needs_confirmation(refused: &autonomy::Refusal) -> Value {
    let mut result = refusal(
        &refused.tool,
        autonomy::NEEDS_CONFIRMATION,
        &refused.message(),
    );
    let details = &mut result["structuredContent"]["error"]["details"];
    if let (Some(details), Some(extra)) = (details.as_object_mut(), refused.details().as_object()) {
        details.extend(extra.clone());
    }
    let text = result["structuredContent"].to_string();
    result["content"][0]["text"] = Value::String(text);
    result
}

/// What the guard learned about an element from a result.
#[derive(Clone, Debug, Default)]
pub(super) struct Element {
    pub label: String,
    /// Its role as the driver names it (`textfield`, `AXTextArea`, …).
    pub role: String,
    pub secure: bool,
    /// For a field: the buttons nearest it in the snapshot, nearest first.
    pub near: Vec<String>,
}

impl<D: Desktop> Guard<D> {
    /// Refuses `tool` when it would take a consequential action the person
    /// hasn't allowed for this task.
    pub(super) fn consent(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
    ) -> Result<(), Value> {
        let mode = self.config.autonomy;
        if mode == Autonomy::Yolo {
            return Ok(());
        }
        let app = resolved
            .first()
            .map(|window| window.app.clone())
            .unwrap_or_default();
        let Some((category, target)) = self.consequence(tool, args, resolved) else {
            return Ok(());
        };
        let authorized = self.store.authorized(&self.config.task);
        autonomy::Gate::new(tool, mode, authorized)
            .check(category, &target)
            .map_err(|refused| needs_confirmation(&refused.in_app(&app)))
    }

    /// The consequential action `tool` would take, and what it targets.
    fn consequence(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
    ) -> Option<(Category, String)> {
        let element = text(args, "ref_id")
            .and_then(|reference| self.learned.elements.lock().get(&reference).cloned());
        match tool.strip_prefix("desktop_").unwrap_or(tool) {
            "click" | "double_click" | "triple_click" => {
                let label = element?.label;
                autonomy::classify(&label).map(|category| (category, format!("“{label}”")))
            }
            "press" => {
                let combo = text(args, "combo")?;
                self.shortcut(&combo, resolved)
            }
            "type" | "set_value" => {
                let key = if tool.ends_with("type") {
                    "text"
                } else {
                    "value"
                };
                // Untrimmed: the Return at the end is the point.
                let typed = args.get(key).and_then(Value::as_str).unwrap_or_default();
                if !(typed.contains('\n') || typed.contains('\r')) {
                    return None;
                }
                let target = self.typed_into(element, resolved);
                submits(&target, false).map(|(category, label)| {
                    (
                        category,
                        format!("text ending in Return, which presses “{label}”"),
                    )
                })
            }
            _ => None,
        }
    }

    /// What Return ends up pressing when text is typed into `field` (a ref
    /// the snapshot showed), or into the focused element without one.
    fn typed_into(&mut self, field: Option<Element>, resolved: &[Resolved]) -> ReturnTarget {
        let pid = self.pid_of(resolved);
        let Some(field) = field else {
            return pid
                .map(|pid| self.screen.return_target(pid))
                .unwrap_or_default();
        };
        let mut target = ReturnTarget::of_role(&field.role);
        // Text goes into it, whatever its role says.
        if !target.editable {
            target.editable = true;
            target.multiline = None;
        }
        target.near = field.near;
        if target.multiline != Some(true) {
            target.default_button = pid
                .map(|pid| self.screen.return_target(pid))
                .and_then(|focused| focused.default_button);
        }
        target
    }

    /// What `combo` does in the app the call targets: its menu item with
    /// that key equivalent, else for Return what Return presses there.
    fn shortcut(&mut self, combo: &str, resolved: &[Resolved]) -> Option<(Category, String)> {
        let (modifiers, key) = combo_parts(combo);
        if !is_command(&modifiers, &key) {
            return None;
        }
        let pid = self.pid_of(resolved)?;
        let items = self.screen.menu_keys(pid);
        if let Some(item) = menu_item(&items, &modifiers, &key) {
            let title = item.title.clone();
            return autonomy::classify(&title)
                .map(|category| (category, format!("the {combo} shortcut (“{title}”)")));
        }
        if key != "return" {
            return None;
        }
        let target = self.screen.return_target(pid);
        submits(&target, !modifiers.is_empty())
            .map(|(category, label)| (category, format!("{combo}, which presses “{label}”")))
    }

    /// The process of the app a call targets.
    fn pid_of(&mut self, resolved: &[Resolved]) -> Option<i64> {
        let first = resolved.first()?;
        let windows = self.list(Look::Check).ok()?;
        first
            .id
            .as_ref()
            .and_then(|id| windows.iter().find(|window| &window.id == id))
            .filter(|window| window.pid.is_some())
            .or_else(|| {
                windows
                    .iter()
                    .find(|window| window.pid.is_some() && same_app(&window.app, &first.app))
            })
            .and_then(|window| window.pid)
    }

    /// Learns labels, roles, and secure fields from a result's data, and
    /// redacts secure values in it.
    pub(super) fn learn_and_redact(&mut self, result: &mut Value) {
        let Some(mut envelope) = super::super::child::envelope(result) else {
            return;
        };
        let Some(data) = envelope.get_mut("data") else {
            return;
        };
        let mut redacted = false;
        let mut learned = Vec::new();
        walk(data, &mut |node| {
            let secure = is_secure(node);
            if secure {
                for key in ["value", "text"] {
                    if node.get(key).is_some_and(|value| !value.is_null()) {
                        node.insert(key.into(), json!({"redacted": true}));
                        redacted = true;
                    }
                }
            }
            if let Some(reference) = node.get("ref_id").and_then(Value::as_str) {
                learned.push((
                    reference.to_owned(),
                    Element {
                        label: label_of(node),
                        role: role_of(node).to_owned(),
                        secure,
                        near: Vec::new(),
                    },
                ));
            }
        });
        let mut near = near_controls(data);
        {
            let mut elements = self.learned.elements.lock();
            for (reference, mut element) in learned {
                element.near = near.remove(&reference).unwrap_or_default();
                elements.insert(reference, element);
            }
        }
        if redacted {
            if result.get("structuredContent").is_some() {
                result["structuredContent"] = envelope.clone();
            }
            if let Some(blocks) = result.get_mut("content").and_then(Value::as_array_mut) {
                for block in blocks.iter_mut() {
                    let is_envelope = block["text"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        .is_some_and(|value| value.get("ok").is_some());
                    if is_envelope {
                        block["text"] = Value::String(envelope.to_string());
                    }
                }
            }
        }
    }

    /// Refuses reading a secure field's value.
    pub(super) fn secure_read(&self, tool: &str, args: &Map<String, Value>) -> Result<(), Value> {
        let (key, reads_value) = match tool {
            "desktop_get" => {
                let property = text(args, "property").unwrap_or_else(|| "text".into());
                ("ref_id", matches!(property.as_str(), "text" | "value"))
            }
            "desktop_wait" => (
                "element",
                text(args, "predicate").as_deref() == Some("value"),
            ),
            _ => return Ok(()),
        };
        let secure = text(args, key)
            .and_then(|reference| {
                self.learned
                    .elements
                    .lock()
                    .get(&reference)
                    .map(|element| element.secure)
            })
            .unwrap_or(false);
        if secure && reads_value {
            return Err(refusal(
                tool,
                "POLICY_DENIED",
                "That is a secure text field (a password): no task may read its value.",
            ));
        }
        Ok(())
    }
}

/// What Return would press, by what it is pressed in (`modified`: with ⌘ or
/// another modifier), and that control's label.
///
/// - Outside a field and in a one-line field it presses the window's
///   default button; a one-line field without one may still sit beside a
///   send-like control (a chat's input).
/// - In a field of many lines it is a new line, unless a send-like control
///   sits right beside it (a chat's composer); a modified Return there
///   submits, to the nearest send-like control or the default button.
fn submits(target: &ReturnTarget, modified: bool) -> Option<(Category, String)> {
    let default = || {
        let label = target.default_button.as_ref()?;
        autonomy::classify(label).map(|category| (category, label.clone()))
    };
    let send_like = || {
        target.near.iter().find_map(|label| {
            autonomy::classify(label)
                .filter(|category| matches!(category, Category::Send | Category::Publish))
                .map(|category| (category, label.clone()))
        })
    };
    if !target.editable {
        return default();
    }
    match target.multiline {
        Some(false) => default().or_else(send_like),
        Some(true) if !modified => send_like(),
        _ => send_like().or_else(default),
    }
}

/// Whether a combo may be an app's command rather than typing: with ⌘,
/// Ctrl, or Option, or a key that acts by itself (⌫, ⌦, Return).
fn is_command(modifiers: &[&str], key: &str) -> bool {
    modifiers
        .iter()
        .any(|modifier| matches!(*modifier, "cmd" | "ctrl" | "option" | "super"))
        || matches!(key, "delete" | "forward_delete" | "return")
}

/// The enabled menu item whose key equivalent is exactly this combo.
fn menu_item<'a>(items: &'a [MenuKey], modifiers: &[&str], key: &str) -> Option<&'a MenuKey> {
    items
        .iter()
        .find(|item| item.enabled && item.key == key && item.modifiers == modifiers)
}

/// A combo's modifiers (sorted) and key, with aliases folded as macOS
/// names them: `Cmd+Shift+D` → (["cmd", "shift"], "d"), Backspace is
/// `delete` (⌫), Enter is `return`.
fn combo_parts(combo: &str) -> (Vec<&'static str>, String) {
    let mut modifiers = Vec::new();
    let mut key = String::new();
    for part in combo.split('+').map(|part| part.trim().to_lowercase()) {
        match part.as_str() {
            "cmd" | "command" | "meta" | "⌘" => modifiers.push("cmd"),
            "shift" | "⇧" => modifiers.push("shift"),
            "option" | "opt" | "alt" | "⌥" => modifiers.push("option"),
            "ctrl" | "control" | "⌃" => modifiers.push("ctrl"),
            "super" | "win" | "windows" => modifiers.push("super"),
            "backspace" | "delete" | "del" | "⌫" => key = "delete".into(),
            "forwarddelete" | "forward_delete" | "⌦" => key = "forward_delete".into(),
            "return" | "enter" | "↩" | "⏎" => key = "return".into(),
            "esc" | "escape" | "⎋" => key = "escape".into(),
            other => key = other.to_owned(),
        }
    }
    modifiers.sort_unstable();
    modifiers.dedup();
    (modifiers, key)
}

/// Visits every object in `value`.
fn walk(value: &mut Value, visit: &mut impl FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(fields) => {
            visit(fields);
            for field in fields.values_mut() {
                walk(field, visit);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| walk(item, visit)),
        _ => {}
    }
}

fn label_of(node: &Map<String, Value>) -> String {
    ["name", "title", "description", "label"]
        .iter()
        .find_map(|key| {
            node.get(*key)
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
        })
        .unwrap_or_default()
        .to_owned()
}

fn role_of(node: &Map<String, Value>) -> &str {
    node.get("role").and_then(Value::as_str).unwrap_or_default()
}

/// A role's letters, lowercase, without Accessibility's `AX`.
fn flat_role(role: &str) -> String {
    let flat: String = role
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    flat.strip_prefix("ax").map(str::to_owned).unwrap_or(flat)
}

/// For each field (a ref whose role takes text) in a snapshot's tree, the
/// labels of the buttons nearest it: its container's, then the next one
/// out (at most three), never the whole window's, nearest first.
fn near_controls(data: &Value) -> HashMap<String, Vec<String>> {
    const LEVELS: usize = 3;
    const MAX_FIELDS: usize = 64;
    const MAX_VISITED: usize = 600;
    struct Node<'a> {
        parent: Option<usize>,
        fields: &'a Map<String, Value>,
        children: Vec<usize>,
    }
    fn collect<'a>(value: &'a Value, parent: Option<usize>, nodes: &mut Vec<Node<'a>>) {
        match value {
            Value::Object(fields) => {
                // Elements have a role; other objects (bounds, a window's
                // id) are looked through.
                let mut here = parent;
                if fields.contains_key("role") {
                    let index = nodes.len();
                    nodes.push(Node {
                        parent,
                        fields,
                        children: Vec::new(),
                    });
                    if let Some(parent) = parent {
                        nodes[parent].children.push(index);
                    }
                    here = Some(index);
                }
                for field in fields.values() {
                    collect(field, here, nodes);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, parent, nodes)),
            _ => {}
        }
    }
    let mut nodes = Vec::new();
    collect(data, None, &mut nodes);
    let mut near = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(reference) = node.fields.get("ref_id").and_then(Value::as_str) else {
            continue;
        };
        if !input_role(role_of(node.fields)).0 {
            continue;
        }
        if near.len() >= MAX_FIELDS {
            break;
        }
        let mut labels: Vec<String> = Vec::new();
        let mut container = index;
        for _ in 0..LEVELS {
            let Some(parent) = nodes[container].parent else {
                break;
            };
            // The window itself (the tree's root) is too far: its toolbar
            // isn't this field's.
            let role = flat_role(role_of(nodes[parent].fields));
            if nodes[parent].parent.is_none() || matches!(role.as_str(), "window" | "sheet") {
                break;
            }
            let mut stack = nodes[parent].children.clone();
            let mut visited = 0;
            while let Some(child) = stack.pop() {
                visited += 1;
                if visited > MAX_VISITED {
                    break;
                }
                if child == index {
                    continue;
                }
                let fields = nodes[child].fields;
                if flat_role(role_of(fields)).contains("button") {
                    let label = label_of(fields);
                    if !label.is_empty() && !labels.contains(&label) {
                        labels.push(label);
                    }
                } else {
                    stack.extend(nodes[child].children.iter().rev());
                }
            }
            container = parent;
        }
        near.insert(reference.to_owned(), labels);
    }
    near
}

/// A password field: agent-desktop's role or states say secure.
fn is_secure(node: &Map<String, Value>) -> bool {
    let says = |key: &str| {
        node.get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| text.to_ascii_lowercase().contains("secure"))
    };
    says("role")
        || says("subrole")
        || node.get("protected").and_then(Value::as_bool) == Some(true)
        || node
            .get("states")
            .and_then(Value::as_array)
            .is_some_and(|states| {
                states.iter().any(|state| {
                    state
                        .as_str()
                        .is_some_and(|state| state.eq_ignore_ascii_case("secure"))
                })
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, modifiers: &[&'static str], key: &str) -> MenuKey {
        MenuKey {
            title: title.into(),
            modifiers: modifiers.to_vec(),
            key: key.into(),
            enabled: true,
        }
    }

    /// Menus of made-up apps: the same keys mean different things.
    #[test]
    fn a_shortcut_means_what_the_apps_menu_says() {
        let mail = [
            item("Send", &["cmd", "shift"], "d"),
            item("Delete", &[], "delete"),
            item("Copy", &["cmd"], "c"),
        ];
        let files = [
            item("Move to Trash", &["cmd"], "delete"),
            item("Empty Trash…", &["cmd", "shift"], "delete"),
            item("Empty Trash", &["cmd", "option", "shift"], "delete"),
        ];
        let editor = [item("Duplicate", &["cmd", "shift"], "d")];
        let find = |items: &[MenuKey], combo: &str| {
            let (modifiers, key) = combo_parts(combo);
            menu_item(items, &modifiers, &key).and_then(|item| autonomy::classify(&item.title))
        };
        assert_eq!(find(&mail, "Cmd+Shift+D"), Some(Category::Send));
        assert_eq!(find(&mail, "backspace"), Some(Category::Delete));
        assert_eq!(find(&mail, "cmd+c"), None);
        assert_eq!(find(&files, "cmd+backspace"), Some(Category::Delete));
        assert_eq!(find(&files, "cmd+delete"), Some(Category::Delete));
        // A dialog first ("…"): the dialog's own button is asked about.
        assert_eq!(find(&files, "cmd+shift+delete"), None);
        assert_eq!(
            find(&files, "cmd+option+shift+delete"),
            Some(Category::Delete)
        );
        assert_eq!(find(&editor, "cmd+shift+d"), None);
        // A disabled item does nothing.
        let disabled = [MenuKey {
            enabled: false,
            ..item("Send", &["cmd", "shift"], "d")
        }];
        assert_eq!(find(&disabled, "cmd+shift+d"), None);
    }

    #[test]
    fn typing_is_not_a_command() {
        let check = |combo: &str| {
            let (modifiers, key) = combo_parts(combo);
            is_command(&modifiers, &key)
        };
        assert!(check("cmd+shift+d"));
        assert!(check("ctrl+enter"));
        assert!(check("return"));
        assert!(check("Backspace"));
        assert!(!check("a"));
        assert!(!check("shift+a"));
        assert!(!check("tab"));
    }

    #[test]
    fn return_presses_the_default_button_or_the_send_control_beside_a_field() {
        let field = |multiline, default: Option<&str>, near: &[&str]| ReturnTarget {
            editable: true,
            multiline,
            default_button: default.map(str::to_owned),
            near: near.iter().map(|label| (*label).to_owned()).collect(),
        };
        let category = |target: &ReturnTarget, modified| submits(target, modified).map(|(c, _)| c);
        // A one-line field in a dialog whose default button sends or buys.
        assert_eq!(
            category(&field(Some(false), Some("Senden"), &[]), false),
            Some(Category::Send)
        );
        assert_eq!(
            category(&field(Some(false), Some("Jetzt kaufen"), &[]), false),
            Some(Category::Pay)
        );
        assert_eq!(category(&field(Some(false), Some("OK"), &[]), false), None);
        // A chat's composer beside its Send button.
        assert_eq!(
            category(&field(Some(true), None, &["Attach", "Send"]), false),
            Some(Category::Send)
        );
        // A message body: Return is a new line, even with "Delete" near.
        assert_eq!(
            category(&field(Some(true), Some("Send"), &["Delete"]), false),
            None
        );
        // …and ⌘Return there submits.
        assert_eq!(
            category(&field(Some(true), Some("Send"), &[]), true),
            Some(Category::Send)
        );
        // Outside a field: the default button.
        let outside = ReturnTarget {
            default_button: Some("Move to Trash".into()),
            ..ReturnTarget::default()
        };
        assert_eq!(category(&outside, false), Some(Category::Delete));
        // Nothing to tell: like any other action.
        assert_eq!(category(&ReturnTarget::default(), false), None);
    }

    #[test]
    fn the_buttons_near_a_field_are_its_containers_not_the_windows() {
        let tree = json!({"tree": {"role": "window", "children": [
            {"role": "toolbar", "children": [
                {"role": "button", "name": "Send", "ref_id": "@s:e1"},
            ]},
            {"role": "scrollarea", "children": [
                {"role": "textarea", "name": "Message body", "ref_id": "@s:e2"},
            ]},
            {"role": "group", "children": [
                {"role": "group", "children": [
                    {"role": "AXTextArea", "name": "Message", "ref_id": "@s:e3"},
                ]},
                {"role": "AXButton", "description": "Send", "ref_id": "@s:e4"},
            ]},
        ]}});
        let near = near_controls(&tree);
        assert_eq!(near["@s:e2"], Vec::<String>::new());
        assert_eq!(near["@s:e3"], ["Send"]);
        assert!(!near.contains_key("@s:e1"));
    }

    #[test]
    fn secure_fields_are_found_by_role_or_state() {
        let node = |value: Value| value.as_object().cloned().unwrap();
        assert!(is_secure(&node(json!({"role": "securetextfield"}))));
        assert!(is_secure(&node(
            json!({"role": "textfield", "states": ["secure"]})
        )));
        assert!(is_secure(&node(json!({"subrole": "AXSecureTextField"}))));
        assert!(!is_secure(&node(
            json!({"role": "textfield", "states": ["focused"]})
        )));
    }
}
