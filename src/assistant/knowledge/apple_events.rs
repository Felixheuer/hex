//! Why the person's browser didn't answer a script, in words: the real
//! error (osascript's, with its Apple Event code) for the agent and the
//! audit log, what it means, and, where the person can fix it, one line
//! for them ([`FOR_PERSON`]) that the overlay shows as a step
//! (`provider::desktop`): Automation in System Settings, JavaScript from
//! Apple Events in the browser, or opening the browser.

use super::browser::{Browser, Family};

/// Starts the line meant for the person in a tool's text.
pub const FOR_PERSON: &str = "For the person: ";

/// What went wrong, read from the error's code and words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trouble {
    /// macOS doesn't let Hex control the browser (-1743).
    NotAllowed,
    /// No answer in time: macOS may be asking the person, or the browser is busy.
    NoAnswer,
    /// The browser refuses JavaScript from Apple Events.
    JavaScriptOff,
    /// The browser isn't running (-600, -609).
    NotRunning,
    /// Not found or not installed (-10814, -1728 on the app).
    NotFound,
    Other,
}

/// The trouble `error` (osascript's text, or the helper's) names.
pub fn classify(error: &str) -> Trouble {
    let lower = error.to_lowercase();
    if lower.contains("-1743")
        || lower.contains("not authorized")
        || lower.contains("not allowed to send apple events")
    {
        Trouble::NotAllowed
    } else if lower.contains("javascript")
        && (lower.contains("turned off")
            || lower.contains("allow javascript")
            || lower.contains("enable the"))
    {
        Trouble::JavaScriptOff
    } else if lower.contains("took longer")
        || lower.contains("-1712")
        || lower.contains("timed out")
    {
        Trouble::NoAnswer
    } else if lower.contains("-600")
        || lower.contains("-609")
        || lower.contains("isn't running")
        || lower.contains("is not running")
    {
        Trouble::NotRunning
    } else if lower.contains("-10814")
        || lower.contains("can't be found")
        || lower.contains("can’t be found")
    {
        Trouble::NotFound
    } else {
        Trouble::Other
    }
}

/// Where the browser's own switch for JavaScript from Apple Events is.
fn javascript_switch(browser: &Browser) -> String {
    match browser.family {
        Family::Safari => format!(
            "in {}: Settings › Advanced › “Show features for web developers”, then Develop › “Allow JavaScript from Apple Events”",
            browser.name
        ),
        Family::Chromium => format!(
            "in {}: View › Developer › “Allow JavaScript from Apple Events”",
            browser.name
        ),
    }
}

/// The person's fix for `trouble` in `browser`, if they have one.
pub fn fix(trouble: Trouble, browser: &Browser) -> Option<String> {
    let name = &browser.name;
    Some(match trouble {
        Trouble::NotAllowed => format!(
            "Allow Hex to control {name}: System Settings › Privacy & Security › Automation › Hex › {name}."
        ),
        Trouble::NoAnswer => format!(
            "If macOS asks whether Hex may control {name}, click Allow (else System Settings › Privacy & Security › Automation › Hex › {name}); then ask again."
        ),
        Trouble::JavaScriptOff => format!(
            "Turn on JavaScript from Apple Events {}.",
            javascript_switch(browser)
        ),
        Trouble::NotRunning => format!("Open {name}, then ask again."),
        Trouble::NotFound | Trouble::Other => return None,
    })
}

/// `error` for the agent: what it means, the real error, and the
/// person's fix on its own line.
pub fn explain(error: &str, browser: &Browser) -> String {
    let trouble = classify(error);
    let name = &browser.name;
    let meaning = match trouble {
        Trouble::NotAllowed => format!("macOS doesn't let Hex control {name} (Automation)"),
        Trouble::NoAnswer => format!(
            "{name} didn't answer in time: macOS may be asking the person whether Hex may control it, or it is busy"
        ),
        Trouble::JavaScriptOff => format!("{name} doesn't allow JavaScript from Apple Events"),
        Trouble::NotRunning => format!("{name} isn't running"),
        Trouble::NotFound => format!("{name} isn't installed or didn't answer"),
        Trouble::Other => format!("{name} didn't answer the script"),
    };
    let error = crate::assistant::store::shorten(error.trim(), 400);
    match fix(trouble, browser) {
        Some(fix) => format!(
            "{meaning}. The error: {error}\nTell the person how to fix it, then try again.\n{FOR_PERSON}{fix}"
        ),
        None => format!("{meaning}. The error: {error}"),
    }
}

/// The line for the person in a tool's `output` (plain text, or its MCP
/// content as JSON, where line breaks are escaped).
pub fn person_fix(output: &str) -> Option<String> {
    let start = output.find(FOR_PERSON)? + FOR_PERSON.len();
    let rest = &output[start..];
    let end = ["\\n", "\n", "\""]
        .iter()
        .filter_map(|stop| rest.find(stop))
        .min()
        .unwrap_or(rest.len());
    let line = rest[..end]
        .trim()
        .replace("\\u201c", "“")
        .replace("\\u201d", "”");
    (!line.is_empty()).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> Browser {
        Browser {
            bundle_id: "com.google.Chrome".into(),
            name: "Google Chrome".into(),
            family: Family::Chromium,
        }
    }

    /// What osascript prints in each case (from real runs, not run here).
    const NOT_ALLOWED: &str = "execution error: Error: Error: Not authorized to send Apple events to Google Chrome. (-1743)";
    const JS_OFF: &str = "execution error: Error: Error: Google Chrome got an error: Executing JavaScript through AppleScript is turned off. To turn it on, from the menu bar, go to View > Developer > Allow JavaScript from Apple Events. (12)";
    const SAFARI_JS_OFF: &str = "execution error: Error: Error: Safari got an error: You must enable the 'Allow JavaScript from Apple Events' option in Safari's Develop menu to use 'do JavaScript'. (8)";
    const NOT_RUNNING: &str = "execution error: Error: Error: Application isn't running. (-600)";
    const TIMEOUT: &str = "\"/usr/bin/osascript\" took longer than 3000 ms";
    const AE_TIMEOUT: &str = "execution error: Error: Error: AppleEvent timed out. (-1712)";
    const NOT_FOUND: &str = "execution error: Error: Error: Application can't be found. (-2700)";

    #[test]
    fn each_error_maps_to_its_trouble_and_fix() {
        assert_eq!(classify(NOT_ALLOWED), Trouble::NotAllowed);
        assert_eq!(classify(JS_OFF), Trouble::JavaScriptOff);
        assert_eq!(classify(SAFARI_JS_OFF), Trouble::JavaScriptOff);
        assert_eq!(classify(NOT_RUNNING), Trouble::NotRunning);
        assert_eq!(classify(TIMEOUT), Trouble::NoAnswer);
        assert_eq!(classify(AE_TIMEOUT), Trouble::NoAnswer);
        assert_eq!(classify(NOT_FOUND), Trouble::NotFound);
        assert_eq!(classify("\"/usr/bin/osascript\" failed"), Trouble::Other);

        let text = explain(NOT_ALLOWED, &chrome());
        assert!(text.contains("(-1743)"), "the real error stays: {text}");
        assert_eq!(
            person_fix(&text).as_deref(),
            Some(
                "Allow Hex to control Google Chrome: System Settings › Privacy & Security › Automation › Hex › Google Chrome."
            )
        );
        let js = explain(JS_OFF, &chrome());
        assert!(
            person_fix(&js)
                .unwrap()
                .contains("View › Developer › “Allow JavaScript from Apple Events”")
        );
        let safari = Browser {
            bundle_id: "com.apple.Safari".into(),
            name: "Safari".into(),
            family: Family::Safari,
        };
        assert!(
            person_fix(&explain(SAFARI_JS_OFF, &safari))
                .unwrap()
                .contains("Develop ›")
        );
        assert_eq!(
            person_fix(&explain(NOT_RUNNING, &chrome())).as_deref(),
            Some("Open Google Chrome, then ask again.")
        );
        assert!(
            person_fix(&explain(TIMEOUT, &chrome()))
                .unwrap()
                .contains("click Allow")
        );
        let other = explain("\"/usr/bin/osascript\" failed", &chrome());
        assert!(other.contains("osascript\" failed") && person_fix(&other).is_none());
    }

    #[test]
    fn the_person_s_line_is_found_in_mcp_content_too() {
        let text = explain(NOT_RUNNING, &chrome());
        let content = serde_json::json!([{"type": "text", "text": text}]).to_string();
        assert_eq!(
            person_fix(&content).as_deref(),
            Some("Open Google Chrome, then ask again.")
        );
        assert_eq!(person_fix("all fine"), None);
    }
}
