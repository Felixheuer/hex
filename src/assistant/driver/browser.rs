//! The person's own browser through Cua Driver (`guard::cua::browser`):
//! Chrome, Edge, or Brave attached over DevTools, so a task reads and acts
//! in a tab of theirs by page refs, in the background. Attaching their
//! signed-in profile is the person's choice, made once in setup (`agent-desk
//! assistant driver browser --attach`, Chrome asks them too); it is kept as
//! `<cua-driver>/browser-attach`, and only then do tasks' drivers start with
//! Cua's `--grant existing-profile`. Without it a task uses their tabs
//! through `hex_browser_tab` (JavaScript from Apple Events), as before: the
//! capability is checked, not assumed.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::cua::Home;

/// The browsers Cua Driver's typed browser tools attach to.
pub const BROWSERS: &[&str] = &[
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "com.brave.Browser",
];

fn marker(home: &Home) -> PathBuf {
    home.path().join("browser-attach")
}

/// Whether the person allowed attaching their browser.
pub fn allowed(home: &Home) -> bool {
    marker(home).is_file()
}

/// Allows (or stops allowing) attaching the person's browser.
pub fn allow(home: &Home, on: bool) -> std::io::Result<()> {
    if on {
        std::fs::create_dir_all(home.path())?;
        std::fs::write(marker(home), "existing-profile\n")
    } else {
        match std::fs::remove_file(marker(home)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// The arguments tasks' drivers get beside `mcp --direct`.
pub fn grants(home: Option<&Home>) -> Vec<String> {
    if home.is_some_and(allowed) {
        vec!["--grant".into(), "existing-profile".into()]
    } else {
        Vec::new()
    }
}

/// One line for `driver status`.
pub fn line(home: &Home) -> String {
    if allowed(home) {
        "person's browser: Chrome, Edge, or Brave through Cua Driver (desktop_browser); \
         hex_browser_tab where it isn't attached"
            .into()
    } else {
        "person's browser: hex_browser_tab (JavaScript from Apple Events); \
         `agent-desk assistant driver browser --attach` lets tasks use Cua Driver's browser tools"
            .into()
    }
}

/// Setup's attach: allows it, then attaches the first window of a running
/// Chrome, Edge, or Brave through a driver started as tasks start it (the
/// browser may ask the person; the app they were in comes back). What
/// happened, as lines.
pub fn attach(home: &Home) -> Result<Vec<String>, String> {
    allow(home, true).map_err(|error| error.to_string())?;
    let path = home.installed().ok_or("Cua Driver isn't installed")?;
    let mut driver = crate::assistant::guard::Driver::start(path)?;
    let mut lines = vec!["Tasks may attach your browser through Cua Driver now.".to_owned()];
    let apps = driver.call("list_apps", json!({}))?;
    let running: Vec<(String, i64)> = apps["structuredContent"]["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|app| app["running"] == true)
        .filter(|app| {
            app["bundle_id"]
                .as_str()
                .is_some_and(|id| BROWSERS.contains(&id))
        })
        .filter_map(|app| Some((app["name"].as_str()?.to_owned(), app["pid"].as_i64()?)))
        .collect();
    let Some((name, pid)) = running.first().cloned() else {
        lines.push(
            "No Chrome, Edge, or Brave runs now; a task attaches one when it needs it.".into(),
        );
        return Ok(lines);
    };
    let windows = driver.call("list_windows", json!({"pid": pid}))?;
    let Some(window) = windows["structuredContent"]["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|window| window["layer"].as_i64().unwrap_or(0) == 0)
        .and_then(|window| window["window_id"].as_i64())
    else {
        lines.push(format!("{name} has no window to attach now."));
        return Ok(lines);
    };
    let bound = driver.call(
        "get_browser_state",
        json!({"pid": pid, "window_id": window}),
    )?;
    if bound["isError"] != true {
        lines.push(format!("{name} is attached."));
        return Ok(lines);
    }
    let prepared = driver.call(
        "browser_prepare",
        json!({"pid": pid, "window_id": window, "strategy": {"kind": "existing_profile"},
               "delivery_mode": "foreground"}),
    )?;
    if prepared["isError"] == true {
        lines.push(format!(
            "{name} didn't attach: {}",
            text_of(&prepared).lines().next().unwrap_or("refused")
        ));
    } else {
        lines.push(format!("{name} is attached."));
    }
    Ok(lines)
}

fn text_of(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_get_the_profile_grant_only_once_the_person_allowed_it() {
        let dir = std::env::temp_dir().join(format!("hex-cua-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = Home(dir.clone());
        assert!(grants(Some(&home)).is_empty());
        assert!(grants(None).is_empty());
        assert!(line(&home).contains("hex_browser_tab"));
        allow(&home, true).unwrap();
        assert_eq!(grants(Some(&home)), ["--grant", "existing-profile"]);
        assert!(line(&home).contains("desktop_browser"));
        allow(&home, false).unwrap();
        allow(&home, false).unwrap();
        assert!(grants(Some(&home)).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
