//! Consequential actions and secure fields. Before a click, a key combo, or
//! text ending in Return, the guard asks what it would do (`autonomy`):
//! the element's own label as the snapshot showed it ("Send", "Move to
//! Trash", "Jetzt kaufen"), or a known shortcut in a known app (⌘⌫ in
//! Finder, ⌘⇧D in Mail, Return in Messages; on Linux and Windows Ctrl
//! stands for ⌘, and Delete or Shift+Delete in Explorer, Nautilus, or
//! Dolphin, Ctrl+Enter in Outlook or Thunderbird count too). Unless the person's mode and
//! what they authorized for this task allow it, it is refused with
//! `NEEDS_CONFIRMATION`. What a task reads on screen never authorizes
//! anything.
//!
//! Secure text fields (passwords) never give up their value: snapshot and
//! find results have it redacted, and reading it is refused.

use serde_json::{Map, Value, json};

use super::super::child::Desktop;
use super::{Guard, Resolved, refusal, text};
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
    pub secure: bool,
}

const CHAT_APPS: &[&str] = &[
    "messages",
    "nachrichten",
    "whatsapp",
    "telegram",
    "signal",
    "slack",
    "discord",
    "microsoft teams",
    "teams",
    "element",
    "threema",
    "skype",
    "telegram desktop",
    "ms-teams",
];

const MAIL_APPS: &[&str] = &[
    "mail",
    "microsoft outlook",
    "outlook",
    "outlook (new)",
    "new outlook",
    "olk",
    "spark",
    "airmail",
    "mimestream",
    "superhuman",
    "thunderbird",
    "mozilla thunderbird",
    "betterbird",
    "evolution",
    "geary",
    "kmail",
    "mailspring",
];

/// File managers on Linux and Windows, where Delete trashes the selection
/// and Shift+Delete deletes it for good.
const FILE_MANAGERS: &[&str] = &[
    "explorer",
    "file explorer",
    "windows explorer",
    "datei-explorer",
    "nautilus",
    "org.gnome.nautilus",
    "files",
    "dateien",
    "dolphin",
    "nemo",
    "thunar",
    "caja",
    "pcmanfm",
    "pcmanfm-qt",
];

/// Linux and Windows apps where Delete deletes the selection.
const DELETE_KEY_APPS: &[&str] = &["photos", "fotos", "gnome photos", "shotwell", "gwenview"];

/// Apps where ⌘⌫ deletes the selection.
const DELETE_APPS: &[&str] = &[
    "finder",
    "mail",
    "photos",
    "fotos",
    "notes",
    "notizen",
    "reminders",
    "erinnerungen",
    "contacts",
    "kontakte",
    "calendar",
    "kalender",
    "music",
    "musik",
];

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
            .map(|window| window.app.as_str())
            .unwrap_or_default();
        let Some((category, target)) = self.consequence(tool, args, app) else {
            return Ok(());
        };
        let authorized = self.store.authorized(&self.config.task);
        autonomy::Gate::new(tool, mode, authorized)
            .check(category, &target)
            .map_err(|refused| needs_confirmation(&refused.in_app(app)))
    }

    /// The consequential action `tool` would take, and what it targets.
    fn consequence(
        &self,
        tool: &str,
        args: &Map<String, Value>,
        app: &str,
    ) -> Option<(Category, String)> {
        let label = |key: &str| {
            text(args, key)
                .and_then(|reference| self.elements.get(&reference))
                .map(|element| element.label.clone())
                .unwrap_or_default()
        };
        match tool.strip_prefix("desktop_").unwrap_or(tool) {
            "click" | "double_click" | "triple_click" => {
                let label = label("ref_id");
                store_button(app, &label)
                    .or_else(|| autonomy::classify(&label))
                    .map(|category| (category, format!("“{label}”")))
            }
            "press" => {
                let combo = text(args, "combo")?;
                shortcut(app, &combo).map(|category| (category, format!("the {combo} shortcut")))
            }
            "type" | "set_value" => {
                let key = if tool.ends_with("type") {
                    "text"
                } else {
                    "value"
                };
                // Untrimmed: the Return at the end is the point.
                let typed = args.get(key).and_then(Value::as_str).unwrap_or_default();
                let submits = typed.contains('\n') || typed.contains('\r');
                (submits && is_one_of(app, CHAT_APPS))
                    .then(|| (Category::Send, "text ending in Return".to_owned()))
            }
            _ => None,
        }
    }

    /// Learns labels and secure fields from a result's data, and redacts
    /// secure values in it.
    pub(super) fn learn_and_redact(&mut self, result: &mut Value) {
        let Some(mut envelope) = super::super::child::envelope(result) else {
            return;
        };
        let Some(data) = envelope.get_mut("data") else {
            return;
        };
        let mut redacted = false;
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
                let label = ["name", "title", "description"]
                    .iter()
                    .find_map(|key| node.get(*key).and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_owned();
                self.elements
                    .insert(reference.to_owned(), Element { label, secure });
            }
        });
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
            .and_then(|reference| self.elements.get(&reference))
            .is_some_and(|element| element.secure);
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

fn is_one_of(app: &str, apps: &[&str]) -> bool {
    let app = super::super::policy::app_key(app);
    // Windows may name an app by its program (`explorer.exe`).
    let app = app.strip_suffix(".exe").unwrap_or(&app);
    apps.contains(&app)
}

/// The App Store's Get, Install, and price buttons.
fn store_button(app: &str, label: &str) -> Option<Category> {
    if super::super::policy::app_key(app) != "app store" {
        return None;
    }
    let label = label.trim().to_lowercase();
    let price = label.chars().any(|c| c.is_ascii_digit())
        && label
            .chars()
            .any(|c| matches!(c, '€' | '$' | '£' | '¥' | '₣'));
    (price || matches!(label.as_str(), "get" | "laden" | "install" | "installieren"))
        .then_some(Category::Pay)
}

/// A combo's modifiers (sorted) and key, with aliases folded:
/// `Cmd+Shift+D` → (["cmd", "shift"], "d").
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
            "backspace" | "⌫" => key = "backspace".into(),
            "delete" | "del" => key = "delete".into(),
            "forwarddelete" | "forward_delete" | "⌦" => key = "forward_delete".into(),
            "return" | "enter" | "↩" => key = "return".into(),
            other => key = other.to_owned(),
        }
    }
    modifiers.sort_unstable();
    modifiers.dedup();
    (modifiers, key)
}

/// What a shortcut does in a known app, on this system.
fn shortcut(app: &str, combo: &str) -> Option<Category> {
    shortcut_on(cfg!(target_os = "macos"), app, combo)
}

/// What a shortcut does in a known app: macOS's shortcuts on a Mac (where
/// "delete" is ⌫), else Linux's and Windows' (Ctrl for ⌘, "delete" the
/// forward Delete key).
fn shortcut_on(mac: bool, app: &str, combo: &str) -> Option<Category> {
    let (modifiers, key) = combo_parts(combo);
    if mac {
        let key = if key == "backspace" {
            "delete".into()
        } else {
            key
        };
        return mac_shortcut(app, modifiers.as_slice(), &key);
    }
    let mut modifiers: Vec<&str> = modifiers
        .into_iter()
        .map(|modifier| if modifier == "ctrl" { "cmd" } else { modifier })
        .collect();
    modifiers.sort_unstable();
    modifiers.dedup();
    let key = if key == "forward_delete" {
        "delete".into()
    } else {
        key
    };
    other_shortcut(app, modifiers.as_slice(), &key)
}

fn mac_shortcut(app: &str, modifiers: &[&str], key: &str) -> Option<Category> {
    match (modifiers, key) {
        ([], "return") | (["cmd"], "return") if is_one_of(app, CHAT_APPS) => Some(Category::Send),
        (["cmd", "shift"], "d") | (["cmd"], "return") if is_one_of(app, MAIL_APPS) => {
            Some(Category::Send)
        }
        (["cmd"], "delete") if is_one_of(app, DELETE_APPS) => Some(Category::Delete),
        (["cmd", "shift"] | ["cmd", "option", "shift"], "delete")
            if is_one_of(app, &["finder"]) =>
        {
            Some(Category::Delete)
        }
        _ => None,
    }
}

/// Linux and Windows, with Ctrl folded into `cmd`.
fn other_shortcut(app: &str, modifiers: &[&str], key: &str) -> Option<Category> {
    match (modifiers, key) {
        ([], "return") | (["cmd"], "return") if is_one_of(app, CHAT_APPS) => Some(Category::Send),
        // Ctrl+Enter sends in Outlook and Thunderbird; Alt+S in Outlook.
        (["cmd"], "return") | (["option"], "s") if is_one_of(app, MAIL_APPS) => {
            Some(Category::Send)
        }
        ([] | ["shift"], "delete")
            if is_one_of(app, FILE_MANAGERS)
                || is_one_of(app, MAIL_APPS)
                || is_one_of(app, DELETE_KEY_APPS) =>
        {
            Some(Category::Delete)
        }
        // Ctrl+D deletes a message in Outlook and Evolution.
        (["cmd"], "d") if is_one_of(app, MAIL_APPS) => Some(Category::Delete),
        // Ctrl+Delete moves to the trash in Nautilus.
        (["cmd"], "delete") if is_one_of(app, FILE_MANAGERS) => Some(Category::Delete),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shortcuts_in_known_apps_are_consequential() {
        let shortcut = |app, combo| shortcut_on(true, app, combo);
        assert_eq!(shortcut("Finder", "cmd+backspace"), Some(Category::Delete));
        assert_eq!(shortcut("Finder", "cmd+delete"), Some(Category::Delete));
        assert_eq!(
            shortcut("Finder", "Cmd+Shift+Delete"),
            Some(Category::Delete)
        );
        assert_eq!(shortcut("Mail", "cmd+shift+d"), Some(Category::Send));
        assert_eq!(
            shortcut("Microsoft Outlook", "cmd+enter"),
            Some(Category::Send)
        );
        assert_eq!(shortcut("Messages", "return"), Some(Category::Send));
        assert_eq!(shortcut("Nachrichten", "enter"), Some(Category::Send));
        assert_eq!(shortcut("TextEdit", "cmd+backspace"), None);
        assert_eq!(shortcut("TextEdit", "return"), None);
        assert_eq!(shortcut("Mail", "cmd+c"), None);
    }

    #[test]
    fn linux_and_windows_shortcuts_use_ctrl_and_the_delete_key() {
        let shortcut = |app, combo| shortcut_on(false, app, combo);
        for app in ["File Explorer", "explorer.exe", "Nautilus", "Dolphin"] {
            assert_eq!(shortcut(app, "Delete"), Some(Category::Delete), "{app}");
            assert_eq!(
                shortcut(app, "shift+delete"),
                Some(Category::Delete),
                "{app}"
            );
            assert_eq!(shortcut(app, "backspace"), None, "{app}");
        }
        assert_eq!(shortcut("Outlook", "ctrl+enter"), Some(Category::Send));
        assert_eq!(
            shortcut("Thunderbird", "Control+Return"),
            Some(Category::Send)
        );
        assert_eq!(shortcut("Outlook", "alt+s"), Some(Category::Send));
        assert_eq!(shortcut("Outlook", "ctrl+d"), Some(Category::Delete));
        assert_eq!(shortcut("Thunderbird", "del"), Some(Category::Delete));
        assert_eq!(shortcut("Slack", "enter"), Some(Category::Send));
        assert_eq!(
            shortcut("Microsoft Teams", "ctrl+enter"),
            Some(Category::Send)
        );
        assert_eq!(shortcut("Notepad", "delete"), None);
        assert_eq!(shortcut("Outlook", "ctrl+c"), None);
    }

    #[test]
    fn the_app_store_buttons_are_purchases() {
        assert_eq!(store_button("App Store", "Get"), Some(Category::Pay));
        assert_eq!(store_button("App Store", "4,99 €"), Some(Category::Pay));
        assert_eq!(store_button("App Store", "Open"), None);
        assert_eq!(store_button("Safari", "Get"), None);
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
