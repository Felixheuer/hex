//! The one autonomy gate every consequential action passes: the desktop
//! guard's clicks and shortcuts (`guard::proxy::consent`), Hex's own
//! browser (`browser::consent`), the person's browser tab
//! (`knowledge::tab`), and app scripts (`guard::proxy::actions`). Each
//! works out what an action would do ([`Category`]) and on what (a
//! target a person reads); the gate decides with the mode and what the
//! person authorized, and a refusal always has the same
//! `NEEDS_CONFIRMATION` shape, which the brain turns into a question.

use serde_json::{Value, json};

use super::{Authorized, Autonomy, Category};

/// The code of every refusal the person's yes would lift.
pub const NEEDS_CONFIRMATION: &str = "NEEDS_CONFIRMATION";

/// The mode and what this task may do, for one tool.
#[derive(Clone, Debug)]
pub struct Gate {
    tool: String,
    mode: Autonomy,
    authorized: Authorized,
}

/// An action the person has to allow first.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub tool: String,
    pub mode: Autonomy,
    pub category: Category,
    /// What it acts on, as a person reads it ("“Send”", "the ⌘⌫ shortcut").
    pub target: String,
    /// The app it would happen in, when known.
    pub app: Option<String>,
}

impl Gate {
    pub fn new(tool: &str, mode: Autonomy, authorized: Authorized) -> Self {
        Self {
            tool: tool.to_owned(),
            mode,
            authorized,
        }
    }

    pub fn mode(&self) -> Autonomy {
        self.mode
    }

    /// Whether an action of `category` on `target` may run now.
    pub fn check(&self, category: Category, target: &str) -> Result<(), Refusal> {
        if super::allowed(self.mode, category, &self.authorized) {
            return Ok(());
        }
        Err(Refusal {
            tool: self.tool.clone(),
            mode: self.mode,
            category,
            target: target.to_owned(),
            app: None,
        })
    }

    /// [`Self::check`] for a control whose label decides what it does
    /// ([`super::classify`]); a label that means nothing consequential runs.
    pub fn check_label(&self, label: &str, target: &str) -> Result<(), Refusal> {
        match super::classify(label) {
            Some(category) => self.check(category, target),
            None => Ok(()),
        }
    }
}

impl Refusal {
    /// The same refusal, in `app` (none when empty).
    pub fn in_app(mut self, app: &str) -> Self {
        let app = app.trim();
        self.app = (!app.is_empty()).then(|| app.to_owned());
        self
    }

    /// What the agent reads: what it would have done, and that it asks.
    pub fn message(&self) -> String {
        let verb = match self.category {
            Category::Send => "send",
            Category::Delete => "delete",
            Category::Pay => "buy or pay",
            Category::Share => "share",
            Category::Publish => "publish",
        };
        let mode = match self.mode {
            Autonomy::Ask => "ask before every consequential action",
            Autonomy::Smart => "only what the request named",
            Autonomy::Yolo => "yolo",
        };
        format!(
            "This would {verb} ({}{}). The person hasn't allowed that for this task (autonomy: \
             {mode}). Don't retry it or try another way around it: ask them and wait for their \
             yes; Hex refuses it until they confirm.",
            self.target,
            self.app
                .as_deref()
                .map(|app| format!(" in {app}"))
                .unwrap_or_default()
        )
    }

    /// `{category, target, tool, mode, app}`.
    pub fn details(&self) -> Value {
        let mut details = json!({
            "category": self.category,
            "target": self.target,
            "tool": self.tool,
            "mode": self.mode,
        });
        if let Some(app) = &self.app {
            details["app"] = json!(app);
        }
        details
    }

    /// The error object: `{code: NEEDS_CONFIRMATION, message, details}`.
    pub fn error(&self) -> Value {
        json!({
            "code": NEEDS_CONFIRMATION,
            "message": self.message(),
            "details": self.details(),
        })
    }

    /// The whole tool result, for a tool without an envelope of its own.
    pub fn result(&self) -> Value {
        let envelope = json!({"ok": false, "error": self.error()});
        json!({
            "content": [{"type": "text", "text": envelope.to_string()}],
            "structuredContent": envelope,
            "isError": true,
        })
    }

    /// The result's text, for a tool that answers with text.
    pub fn text(&self) -> String {
        json!({"ok": false, "error": self.error()}).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_gate_decides_and_refuses_in_one_shape() {
        let gate = Gate::new("hex_browser_tab", Autonomy::Smart, Authorized::default());
        assert_eq!(gate.mode(), Autonomy::Smart);
        assert!(gate.check_label("Weiter", "“Weiter”").is_ok());
        let refused = gate
            .check_label("Senden", "“Senden”")
            .unwrap_err()
            .in_app("Mail");
        assert_eq!(refused.category, Category::Send);
        let result = refused.result();
        let error = &result["structuredContent"]["error"];
        assert_eq!(error["code"], NEEDS_CONFIRMATION);
        assert_eq!(error["details"]["category"], "send");
        assert_eq!(error["details"]["target"], "“Senden”");
        assert_eq!(error["details"]["app"], "Mail");
        assert_eq!(error["details"]["mode"], "smart");
        assert_eq!(result["isError"], true);
        assert!(refused.message().contains("in Mail"));
        assert!(refused.text().contains(NEEDS_CONFIRMATION));
        assert_eq!(
            Gate::new("t", Autonomy::Smart, Authorized::default())
                .check(Category::Pay, "x")
                .unwrap_err()
                .in_app(" ")
                .app,
            None
        );

        let mut sending = Authorized::default();
        sending.categories.insert(Category::Send);
        let gate = Gate::new("t", Autonomy::Smart, sending.clone());
        assert!(gate.check(Category::Send, "x").is_ok());
        assert!(gate.check(Category::Delete, "x").is_err());
        assert!(
            Gate::new("t", Autonomy::Ask, sending)
                .check(Category::Send, "x")
                .is_err()
        );
        assert!(
            Gate::new("t", Autonomy::Yolo, Authorized::default())
                .check(Category::Pay, "x")
                .is_ok()
        );
    }
}
