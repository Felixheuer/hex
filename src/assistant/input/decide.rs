//! Two decisions, by measurement and never by app name:
//!
//! - where a task's window waits ([`place`]): off the screen when it still
//!   renders and takes input there, else behind the person's windows (never
//!   above them), and never minimized (a minimized window takes no posted
//!   input and gives no picture for the live mini);
//! - whether an action that failed through Accessibility is tried again by
//!   pid-targeted input at the element's spot ([`fallback`]).

use serde_json::Value;

/// Where a window waits while a task works in it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
    /// Hanging off the screen but a sliver: out of sight, still rendering.
    OffScreen,
    /// On the screen, layered under the person's front window.
    Behind,
}

/// What was measured for one window while it hung off the screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Measured {
    /// A picture taken there differed from the one before an action (or a
    /// first picture came at all).
    pub fresh_frame: Option<bool>,
    /// An action there (Accessibility or posted) took effect.
    pub input_took: Option<bool>,
}

/// Where a window waits: off the screen unless measuring showed it stops
/// rendering or taking input there. Unmeasured windows stay off the screen.
pub fn place(measured: Measured) -> Place {
    if measured.fresh_frame == Some(false) || measured.input_took == Some(false) {
        Place::Behind
    } else {
        Place::OffScreen
    }
}

/// What to do after an action through Accessibility failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fallback {
    /// It failed in a way input to the pid can get around: try the same
    /// spot by pid-targeted input (once), before any window comes forward.
    Pointer,
    /// Nothing to retry (it worked, or failed for a reason input can't fix).
    None,
}

/// Accessibility's errors that mean "the app didn't answer in time or
/// couldn't do it now" (`kAXErrorCannotComplete`, `kAXErrorNotImplemented`,
/// `kAXErrorActionUnsupported`, timeouts), as agent-desktop or Cua Driver
/// report them.
const UNREACHABLE: &[&str] = &[
    "-25204",
    "-25205",
    "-25206",
    "kaxerrorcannotcomplete",
    "kaxerrorattributeunsupported",
    "kaxerroractionunsupported",
    "cannot complete",
    "cannotcomplete",
    "timed out",
    "timeout",
    "time out",
    "zeitfehler",
    "not_actionable",
    "action unsupported",
    "no ax action",
];

/// Whether `result` (an MCP tool result) is a failed action that pid input
/// may do instead.
pub fn fallback(result: &Value) -> Fallback {
    let failed = result.get("isError").and_then(Value::as_bool) == Some(true)
        || result
            .get("structuredContent")
            .and_then(|content| content.get("ok"))
            .and_then(Value::as_bool)
            == Some(false);
    if !failed {
        return Fallback::None;
    }
    let text = result.to_string().to_lowercase();
    // Refusals by Hex itself (leases, consent, a pause) are never retried.
    if text.contains("hex_guard") || text.contains("needs_confirmation") {
        return Fallback::None;
    }
    if UNREACHABLE.iter().any(|needle| text.contains(needle)) {
        Fallback::Pointer
    } else {
        Fallback::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_window_waits_off_the_screen_unless_measured_otherwise() {
        assert_eq!(place(Measured::default()), Place::OffScreen);
        let rendering = Measured {
            fresh_frame: Some(true),
            input_took: Some(true),
        };
        assert_eq!(place(rendering), Place::OffScreen);
        let frozen = Measured {
            fresh_frame: Some(false),
            input_took: None,
        };
        assert_eq!(place(frozen), Place::Behind);
        let deaf = Measured {
            fresh_frame: Some(true),
            input_took: Some(false),
        };
        assert_eq!(place(deaf), Place::Behind);
    }

    fn error(code: &str, message: &str) -> Value {
        let envelope = json!({"ok": false, "error": {"code": code, "message": message}});
        json!({"content": [{"type": "text", "text": envelope.to_string()}],
            "structuredContent": envelope, "isError": true})
    }

    #[test]
    fn accessibility_timeouts_and_cannot_complete_fall_back() {
        assert_eq!(
            fallback(&error("ACTION_FAILED", "AXPress failed: -25204")),
            Fallback::Pointer
        );
        assert_eq!(
            fallback(&error("AX_ERROR", "kAXErrorCannotComplete (-25204)")),
            Fallback::Pointer
        );
        assert_eq!(
            fallback(&error("TIMEOUT", "the app timed out")),
            Fallback::Pointer
        );
        assert_eq!(
            fallback(
                &json!({"content": [{"type": "text", "text": "AXError -25205"}], "isError": true})
            ),
            Fallback::Pointer
        );
    }

    #[test]
    fn successes_hex_refusals_and_other_errors_do_not() {
        let ok = json!({"structuredContent": {"ok": true, "data": {}}, "isError": false});
        assert_eq!(fallback(&ok), Fallback::None);
        assert_eq!(
            fallback(&error("STALE_REF", "take a new snapshot")),
            Fallback::None
        );
        let refused = json!({"structuredContent": {"ok": false, "error": {"code": "NEEDS_CONFIRMATION",
            "message": "timeout", "details": {"kind": "hex_guard"}}}, "isError": true});
        assert_eq!(fallback(&refused), Fallback::None);
    }
}
