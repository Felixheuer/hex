//! The autonomy mode, enforced on the browser: before a click, an Enter or
//! ⌘↩ press, or typing that ends a line, the proxy works out what the
//! action targets (the element's role and name from the session's last
//! snapshot, or the page itself), classifies it (`autonomy::classify`),
//! and refuses with `NEEDS_CONFIRMATION` when the mode and the task's
//! authorization don't allow it. The brain turns that into a question.
//! Text on a page can't authorize anything: only the person's request and
//! answers do (`autonomy::read`).
//!
//! Also refused outside YOLO: scripts that click, submit, or send requests
//! (the task is told to use the click and fill tools instead), and saved
//! flows that would write.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::assistant::autonomy::{self, Authorized, Autonomy, Category};

/// What a session's page last showed, from the tool results passing back.
#[derive(Clone, Debug, Default)]
struct Seen {
    /// `e7` → (role, name).
    refs: HashMap<String, (String, String)>,
    /// The snapshot's text, for recipients.
    text: String,
    origin: String,
}

/// The element in focus (or at a selector) and the controls around it, as
/// [`INSPECT`] reports them.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Inspection {
    pub found: bool,
    pub label: String,
    pub editable: bool,
    pub multiline: bool,
    /// The submit controls of the form it's in.
    pub form: Vec<String>,
    /// The controls nearest it (its closest container that has any).
    pub near: Vec<String>,
    pub host: String,
    pub title: String,
}

/// Reads the page of a session, read-only.
pub trait Probe: Send {
    /// The focused element after focusing `selector` (when given).
    fn inspect(&self, session: &str, focus: Option<&str>) -> Option<Inspection>;
    /// An element's accessible label or text.
    fn label(&self, session: &str, selector: &str) -> Option<String>;
}

/// The mode and what this task may do, read fresh on every check so a
/// change in Settings or a confirmation counts at once.
pub type Policy = Box<dyn Fn() -> (Autonomy, Authorized) + Send>;

pub struct Consent {
    seen: HashMap<String, Seen>,
    probe: Box<dyn Probe>,
    policy: Policy,
}

/// What a call does that may be consequential.
#[derive(Debug, PartialEq)]
enum Act {
    Click(String),
    /// Enter (or a line typed) at the focused element or this selector.
    Enter(Option<String>),
    /// ⌘↩ or Ctrl+↩: send in Gmail, Outlook, and most chat apps.
    SendShortcut,
    /// Delete, Backspace, or Gmail's `#` outside a text field.
    DeleteKey,
}

impl Consent {
    pub fn new(probe: Box<dyn Probe>, policy: Policy) -> Self {
        Self {
            seen: HashMap::new(),
            probe,
            policy,
        }
    }

    /// Learns from a tool result for `session`: its snapshot's refs.
    pub fn observe(&mut self, session: &str, result: &Value) {
        let data = &result["structuredContent"]["response"]["data"];
        let Some(refs) = data.get("refs").and_then(Value::as_object) else {
            return;
        };
        let seen = self.seen.entry(session.to_owned()).or_default();
        seen.refs = refs
            .iter()
            .map(|(id, info)| {
                let text = |key| info.get(key).and_then(Value::as_str).unwrap_or_default();
                (
                    id.clone(),
                    (text("role").to_owned(), text("name").to_owned()),
                )
            })
            .collect();
        seen.text = data
            .get("snapshot")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(origin) = data.get("origin").and_then(Value::as_str) {
            seen.origin = origin.to_owned();
        }
    }

    /// Whether `tool` may run with `arguments` (already pinned to a
    /// session); `Err` is the tool result to answer with instead.
    pub fn check(&mut self, tool: &str, arguments: &mut Map<String, Value>) -> Result<(), Value> {
        let (mode, authorized) = (self.policy)();
        if mode == Autonomy::Yolo {
            return Ok(());
        }
        let session = arguments
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match tool {
            "agent_browser_eval" => {
                let script = arguments
                    .get("script")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if acts(script) {
                    return Err(plain_error(
                        "Scripts may read the page but not click, submit, or send. Use the \
                         click, fill, and press tools for that.",
                    ));
                }
                return Ok(());
            }
            "agent_browser_flow_run" => {
                arguments.remove("allowWrite");
                return Ok(());
            }
            _ => {}
        }
        let Some(act) = act(tool, arguments) else {
            return Ok(());
        };
        let Some((category, target)) = self.judge(&session, &act) else {
            return Ok(());
        };
        if autonomy::allowed(mode, category, &authorized) {
            return Ok(());
        }
        Err(needs_confirmation(tool, mode, category, &target))
    }

    /// The category and a person's description of what `act` would do.
    fn judge(&self, session: &str, act: &Act) -> Option<(Category, String)> {
        let seen = self.seen.get(session);
        match act {
            Act::Click(selector) => {
                let label = selector
                    .strip_prefix('@')
                    .and_then(|id| seen?.refs.get(id))
                    .map(|(_, name)| name.clone())
                    .or_else(|| self.probe.label(session, selector))?;
                let category = autonomy::classify(&label)?;
                Some((category, self.describe(session, category, &label)))
            }
            Act::SendShortcut => {
                let page = self.probe.inspect(session, None).unwrap_or_default();
                // Outside a text field it presses whatever has focus.
                let category = if page.editable {
                    Category::Send
                } else {
                    autonomy::classify(&page.label)?
                };
                Some((category, self.describe(session, category, "⌘↩")))
            }
            Act::DeleteKey => {
                let page = self.probe.inspect(session, None).unwrap_or_default();
                (!page.editable).then(|| {
                    let what = if page.label.is_empty() {
                        "the selected items".to_owned()
                    } else {
                        format!("“{}”", page.label)
                    };
                    (
                        Category::Delete,
                        format!("Delete {what} on {}", self.place(session, &page)),
                    )
                })
            }
            Act::Enter(focus) => {
                let page = self.probe.inspect(session, focus.as_deref())?;
                let controls: &[String] = if !page.editable {
                    std::slice::from_ref(&page.label)
                } else if !page.form.is_empty() {
                    &page.form
                } else if page.multiline && is_mail(&page.host) {
                    // A new line in an email body.
                    &[]
                } else {
                    &page.near
                };
                controls.iter().find_map(|label| {
                    let category = autonomy::classify(label)?;
                    Some((category, self.describe(session, category, label)))
                })
            }
        }
    }

    /// "Send email to 3 recipients", "Delete “Q4 Budget” on
    /// docs.google.com", …
    fn describe(&self, session: &str, category: Category, label: &str) -> String {
        let label = &shown(label);
        let seen = self.seen.get(session);
        let place = self.place(session, &Inspection::default());
        if category == Category::Send
            && let Some(count) = seen.map(|seen| recipients(&seen.text)).filter(|n| *n > 0)
        {
            let plural = if count == 1 { "" } else { "s" };
            return format!("Send email to {count} recipient{plural} (“{label}” on {place})");
        }
        let verb = match category {
            Category::Send => "Send",
            Category::Delete => "Delete",
            Category::Pay => "Pay",
            Category::Share => "Share",
            Category::Publish => "Publish",
        };
        format!("{verb}: “{label}” on {place}")
    }

    fn place(&self, session: &str, page: &Inspection) -> String {
        let origin = self
            .seen
            .get(session)
            .map(|seen| seen.origin.as_str())
            .unwrap_or_default();
        let host = origin
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .filter(|host| !host.is_empty())
            .unwrap_or(&page.host);
        if host.is_empty() {
            "the page".into()
        } else {
            host.to_owned()
        }
    }
}

/// The consequential part of a tool call, if any.
fn act(tool: &str, arguments: &Map<String, Value>) -> Option<Act> {
    let text = |key| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match tool {
        "agent_browser_click" => text("selector").map(Act::Click),
        "agent_browser_press" => {
            let key = text("key")?.to_lowercase();
            let parts: Vec<&str> = key.split('+').map(str::trim).collect();
            let main = *parts.last()?;
            let modified = parts[..parts.len() - 1]
                .iter()
                .any(|part| matches!(*part, "meta" | "cmd" | "command" | "control" | "ctrl"));
            match main {
                "enter" | "return" if modified => Some(Act::SendShortcut),
                "enter" | "return" => Some(Act::Enter(None)),
                "delete" | "backspace" | "#" => Some(Act::DeleteKey),
                _ => None,
            }
        }
        // Typing that ends a line presses Enter in that field.
        "agent_browser_type" | "agent_browser_fill" => {
            let typed = text("text")?;
            (typed.contains('\n') || typed.contains('\r')).then(|| Act::Enter(text("selector")))
        }
        _ => None,
    }
}

/// Whether a script acts on the page rather than reading it.
fn acts(script: &str) -> bool {
    [
        ".click(",
        ".submit(",
        "requestsubmit",
        "dispatchevent",
        "fetch(",
        "xmlhttprequest",
        "sendbeacon",
        "new keyboardevent",
        "new mouseevent",
        "new pointerevent",
        ".send(",
    ]
    .iter()
    .any(|pattern| script.to_lowercase().contains(pattern))
}

/// A label as a person reads it: without bidi marks and keyboard hints
/// (Gmail's "Send (⌘Enter)" is "Send").
fn shown(label: &str) -> String {
    let plain: String = label
        .chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect();
    let plain = plain.trim();
    match plain.rfind(" (") {
        Some(at) if plain.ends_with(')') && at > 0 => plain[..at].trim().to_owned(),
        _ => plain.to_owned(),
    }
}

fn is_mail(host: &str) -> bool {
    host == "mail.google.com" || host.starts_with("outlook.") || host.starts_with("mail.")
}

/// Recipients in a snapshot's To/Cc/Bcc fields (`textbox "To recipients"
/// [ref=e4]: a@x.com, b@y.com`), in English, German, French, and Spanish.
fn recipients(snapshot: &str) -> usize {
    snapshot
        .lines()
        .filter_map(|line| {
            let line = line.trim_start().trim_start_matches("- ");
            let rest = line.strip_prefix("textbox \"")?;
            let (name, value) = rest.split_once('"')?;
            let name = name.to_lowercase();
            let field = [
                "to",
                "cc",
                "bcc",
                "an",
                "à",
                "para",
                "recipients",
                "empfänger",
            ]
            .iter()
            .any(|word| name.split_whitespace().next() == Some(word))
                || name.contains("recipient")
                || name.contains("empfänger")
                || name.contains("destinataire")
                || name.contains("destinatario");
            field.then(|| value.matches('@').count())
        })
        .sum()
}

/// A refusal the brain turns into a question for the person.
pub fn needs_confirmation(tool: &str, mode: Autonomy, category: Category, target: &str) -> Value {
    let category_name = serde_json::to_value(category).unwrap_or(Value::Null);
    let mode_name = serde_json::to_value(mode).unwrap_or(Value::Null);
    let envelope = json!({
        "ok": false,
        "error": {
            "code": "NEEDS_CONFIRMATION",
            "message": format!(
                "This needs the person's yes first: {target}. Hex is asking them; don't try \
                 another way around it. Continue with other work, or wait."
            ),
            "details": {
                "category": category_name,
                "target": target,
                "tool": tool,
                "mode": mode_name,
            },
        },
    });
    json!({
        "content": [{"type": "text", "text": envelope.to_string()}],
        "structuredContent": envelope,
        "isError": true,
    })
}

fn plain_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// Reads the focused element and the controls around it, read-only.
pub const INSPECT: &str = r#"(() => {
  const clean = (text) => (text || '').replace(/\s+/g, ' ').trim().slice(0, 120);
  const label = (e) => e ? clean(e.getAttribute('aria-label') || e.getAttribute('data-tooltip') || e.innerText || e.value || e.title || e.getAttribute('alt')) : '';
  const controls = (root) => root ? Array.from(root.querySelectorAll('button,[role=button],input[type=submit],input[type=button]')).map(label).filter(Boolean).slice(0, 24) : [];
  const el = document.activeElement;
  const editable = !!el && (el.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) || el.getAttribute('role') === 'textbox');
  const form = el && el.form ? el.form : (el && el.closest ? el.closest('form') : null);
  let near = [];
  for (let p = el, i = 0; p && i < 8 && near.length === 0; i++) { p = p.parentElement; if (p && p !== document.body) near = controls(p); }
  return {
    found: !!el && el !== document.body,
    label: label(el),
    editable,
    multiline: !!el && (el.isContentEditable || el.tagName === 'TEXTAREA'),
    form: form ? controls(form) : [],
    near,
    host: location.hostname,
    title: clean(document.title),
  };
})()"#;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A real `agent_browser_snapshot` result (agent-browser 0.42.0) of a
    /// page with Gmail's compose buttons, a Sheets-style share dialog, a
    /// shop form, and a search form.
    const SNAPSHOT: &str =
        include_str!("../../../tests/fixtures/assistant/browser-snapshot-actions.json");

    #[derive(Clone, Default)]
    struct Page {
        focused: Option<Inspection>,
        labels: HashMap<String, String>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Probe for Page {
        fn inspect(&self, _: &str, focus: Option<&str>) -> Option<Inspection> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("inspect {focus:?}"));
            self.focused.clone()
        }
        fn label(&self, _: &str, selector: &str) -> Option<String> {
            self.asked.lock().unwrap().push(format!("label {selector}"));
            self.labels.get(selector).cloned()
        }
    }

    fn consent(mode: Autonomy, authorized: &[Category], page: Page) -> Consent {
        let authorized = Authorized {
            categories: authorized.iter().copied().collect(),
            ..Authorized::default()
        };
        let mut consent =
            Consent::new(Box::new(page), Box::new(move || (mode, authorized.clone())));
        consent.observe("hex-t1", &serde_json::from_str(SNAPSHOT).unwrap());
        consent
    }

    fn click(selector: &str) -> Map<String, Value> {
        json!({"selector": selector, "session": "hex-t1"})
            .as_object()
            .cloned()
            .unwrap()
    }

    fn refused(result: Result<(), Value>) -> Value {
        result.expect_err("refused")["structuredContent"]["error"].clone()
    }

    #[test]
    fn gmails_send_button_waits_for_the_person_in_smart_mode() {
        let mut gate = consent(Autonomy::Smart, &[], Page::default());
        // e7: button "Send (⌘Enter)", beside To: a@example.com, b@example.com.
        let error = refused(gate.check("agent_browser_click", &mut click("@e7")));
        assert_eq!(error["code"], "NEEDS_CONFIRMATION");
        assert_eq!(error["details"]["category"], "send");
        let target = error["details"]["target"].as_str().unwrap();
        assert_eq!(
            target,
            "Send email to 2 recipients (“Send” on mail.google.com)"
        );
        // "Senden", the share dialog's "Send" and "Freigeben", "Löschen",
        // "Jetzt kaufen", "Submit order", "Post".
        for (selector, category) in [
            ("@e9", "send"),
            ("@e14", "send"),
            ("@e15", "share"),
            ("@e10", "delete"),
            ("@e11", "delete"),
            ("@e19", "pay"),
            ("@e1", "pay"),
            ("@e2", "publish"),
        ] {
            let error = refused(gate.check("agent_browser_click", &mut click(selector)));
            assert_eq!(error["details"]["category"], category, "{selector}");
        }
        // "Discard draft", "Archive", "Copy link", "Search", a link: no.
        for selector in ["@e8", "@e12", "@e16", "@e21", "@e3"] {
            assert!(
                gate.check("agent_browser_click", &mut click(selector))
                    .is_ok(),
                "{selector}"
            );
        }
    }

    #[test]
    fn what_the_person_asked_for_runs_and_yolo_runs_everything() {
        let mut gate = consent(Autonomy::Smart, &[Category::Send], Page::default());
        assert!(gate.check("agent_browser_click", &mut click("@e7")).is_ok());
        assert!(
            gate.check("agent_browser_click", &mut click("@e10"))
                .is_err()
        );
        let mut ask = consent(Autonomy::Ask, &[Category::Send], Page::default());
        assert!(ask.check("agent_browser_click", &mut click("@e7")).is_err());
        let mut yolo = consent(Autonomy::Yolo, &[], Page::default());
        for selector in ["@e7", "@e10", "@e19"] {
            assert!(
                yolo.check("agent_browser_click", &mut click(selector))
                    .is_ok()
            );
        }
        let mut script = json!({"script": "document.querySelector('[aria-label^=Send]').click()"});
        assert!(
            yolo.check("agent_browser_eval", script.as_object_mut().unwrap())
                .is_ok()
        );
    }

    #[test]
    fn a_selector_without_a_snapshot_is_looked_up_on_the_page() {
        let page = Page {
            labels: HashMap::from([("#buy".to_owned(), "Kaufen".to_owned())]),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], page.clone());
        let error = refused(gate.check("agent_browser_click", &mut click("#buy")));
        assert_eq!(error["details"]["category"], "pay");
        assert_eq!(page.asked.lock().unwrap().as_slice(), ["label #buy"]);
        assert!(
            gate.check("agent_browser_click", &mut click("#other"))
                .is_ok()
        );
    }

    fn press(key: &str) -> Map<String, Value> {
        json!({"key": key, "session": "hex-t1"})
            .as_object()
            .cloned()
            .unwrap()
    }

    #[test]
    fn keys_that_send_submit_or_delete_are_caught() {
        let body = Inspection {
            found: true,
            label: "Message Body".into(),
            editable: true,
            multiline: true,
            near: vec!["Send (⌘Enter)".into(), "Discard draft".into()],
            host: "mail.google.com".into(),
            ..Inspection::default()
        };
        let in_body = Page {
            focused: Some(body.clone()),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], in_body.clone());
        // Gmail's ⌘↩ sends; a plain Enter in the body is a new line.
        for key in ["Meta+Enter", "Control+Enter", "cmd+return"] {
            let error = refused(gate.check("agent_browser_press", &mut press(key)));
            assert_eq!(error["details"]["category"], "send", "{key}");
        }
        assert!(
            gate.check("agent_browser_press", &mut press("Enter"))
                .is_ok()
        );
        assert!(gate.check("agent_browser_press", &mut press("Tab")).is_ok());
        // Enter in a chat box next to a Send button sends.
        let chat = Page {
            focused: Some(Inspection {
                host: "chat.example.com".into(),
                ..body.clone()
            }),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], chat);
        assert!(
            gate.check("agent_browser_press", &mut press("Enter"))
                .is_err()
        );
        // Enter in a shop's quantity field submits its form; in a search
        // field it searches.
        let shop = Page {
            focused: Some(Inspection {
                found: true,
                label: "Quantity".into(),
                editable: true,
                form: vec!["Jetzt kaufen".into()],
                ..Inspection::default()
            }),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], shop.clone());
        let error = refused(gate.check("agent_browser_press", &mut press("Enter")));
        assert_eq!(error["details"]["category"], "pay");
        let mut typed = json!({"selector": "@e18", "text": "2\n", "session": "hex-t1"});
        assert!(
            gate.check("agent_browser_type", typed.as_object_mut().unwrap())
                .is_err()
        );
        assert!(
            shop.asked
                .lock()
                .unwrap()
                .contains(&"inspect Some(\"@e18\")".to_owned())
        );
        let search = Page {
            focused: Some(Inspection {
                found: true,
                label: "Search".into(),
                editable: true,
                form: vec!["Search".into()],
                ..Inspection::default()
            }),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], search);
        assert!(
            gate.check("agent_browser_press", &mut press("Enter"))
                .is_ok()
        );
        // Gmail's `#` and Delete outside a text field delete.
        let list = Page {
            focused: Some(Inspection {
                found: true,
                label: "Conversation from Max".into(),
                ..Inspection::default()
            }),
            ..Page::default()
        };
        let mut gate = consent(Autonomy::Smart, &[], list);
        for key in ["#", "Delete", "Backspace"] {
            let error = refused(gate.check("agent_browser_press", &mut press(key)));
            assert_eq!(error["details"]["category"], "delete", "{key}");
        }
        // In a text field they edit text.
        let mut gate = consent(Autonomy::Smart, &[], in_body);
        assert!(
            gate.check("agent_browser_press", &mut press("Backspace"))
                .is_ok()
        );
    }

    #[test]
    fn scripts_may_read_but_not_act_and_flows_may_not_write() {
        let mut gate = consent(Autonomy::Smart, &[], Page::default());
        let mut read = json!({"script": "document.title"});
        assert!(
            gate.check("agent_browser_eval", read.as_object_mut().unwrap())
                .is_ok()
        );
        for script in [
            "document.querySelector('#send').click()",
            "document.forms[0].requestSubmit()",
            "fetch('/api/send', {method: 'POST'})",
        ] {
            let mut call = json!({ "script": script });
            assert!(
                gate.check("agent_browser_eval", call.as_object_mut().unwrap())
                    .is_err(),
                "{script}"
            );
        }
        let mut flow = json!({"name": "export", "allowWrite": "all"});
        let flow = flow.as_object_mut().unwrap();
        assert!(gate.check("agent_browser_flow_run", flow).is_ok());
        assert!(flow.get("allowWrite").is_none());
    }

    #[test]
    fn recipients_are_counted_in_to_fields() {
        let snapshot = "- textbox \"To recipients\" [ref=e4]: a@example.com, b@example.com\n\
            - textbox \"Cc\" [ref=e5]: c@example.com\n\
            - textbox \"Subject\" [ref=e6]: Re: x@y.com";
        assert_eq!(recipients(snapshot), 3);
        assert_eq!(recipients("- textbox \"An\" [ref=e1]: max@example.de"), 1);
        assert_eq!(recipients("- button \"Send\" [ref=e2]"), 0);
    }
}
