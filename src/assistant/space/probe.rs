//! `agent-desk dev assistant-space-probe` (debug builds): whether parking a window
//! keeps it usable on this Mac. It opens a TextEdit window of its own (a
//! file in a temporary folder), parks it, has agent-desktop read and set its
//! text there, shows it again, and quits TextEdit, printing each step.
//!
//! It visibly opens and moves a window, so it runs only when the person runs
//! it, and it stops at once if TextEdit is already open, so it never touches
//! their documents.

use std::path::Path;
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};
use serde_json::Value;

use super::{bounds, call, park_now, show_now};
use crate::assistant::TaskWindow;
use crate::assistant::backends::{desktop, search};

const APP: &str = "TextEdit";
const TITLE: &str = "Agent-Desk Space Probe";
const BEFORE: &str = "Agent-Desk space probe: before";
const AFTER: &str = "Agent-Desk space probe: after";

/// How the probe went, step by step.
#[derive(Default)]
struct Report {
    steps: Vec<(String, bool)>,
}

impl Report {
    fn step(&mut self, what: &str, ok: bool, detail: impl std::fmt::Display) -> bool {
        println!("  {} {what}: {detail}", if ok { "ok  " } else { "FAIL" });
        self.steps.push((what.to_owned(), ok));
        ok
    }
}

pub fn run() -> Result<()> {
    let tool =
        search::find("agent-desktop").ok_or_else(|| eyre!("agent-desktop isn't installed"))?;
    let grants = call(&tool, &["permissions"]).map_err(|error| eyre!(error))?;
    if !desktop::grants(&grants).is_some_and(|grants| grants.accessibility) {
        return Err(eyre!(
            "agent-desktop has no Accessibility from this terminal; grant it and run again"
        ));
    }
    let apps = desktop::apps(&call(&tool, &["list-apps"]).map_err(|error| eyre!(error))?);
    if apps.iter().any(|app| app == APP) {
        return Err(eyre!(
            "TextEdit is open. Quit it first, so the probe never touches your documents"
        ));
    }
    let folder = std::env::temp_dir().join("hex-space-probe");
    std::fs::create_dir_all(&folder)?;
    let file = folder.join(format!("{TITLE}.txt"));
    std::fs::write(&file, BEFORE)?;
    println!("space probe: opening {} in TextEdit", file.display());
    let mut report = Report::default();
    let result = probe(&tool, &file, &mut report);
    // TextEdit was not running before: quitting it closes only the probe's
    // document (saved as it is, a temporary file).
    let quit = call(&tool, &["close-app", APP]);
    std::thread::sleep(Duration::from_secs(2));
    let still_open = call(&tool, &["list-apps"])
        .map(|data| desktop::apps(&data).iter().any(|app| app == APP))
        .unwrap_or(false);
    report.step(
        "quit TextEdit",
        quit.is_ok() && !still_open,
        if still_open {
            "TextEdit is still open (asking about the document?); close it yourself".to_owned()
        } else {
            quit.err().unwrap_or_else(|| "quit".into())
        },
    );
    let _ = std::fs::remove_dir_all(&folder);
    if let Err(error) = result {
        println!("  stopped: {error}");
    }
    let failed: Vec<_> = report
        .steps
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(what, _)| what.as_str())
        .collect();
    if failed.is_empty() {
        println!("verdict: parking works here; agent-desktop reads and sets a parked window");
    } else {
        println!(
            "verdict: parking does NOT fully work here: {}",
            failed.join(", ")
        );
    }
    Ok(())
}

fn probe(tool: &Path, file: &Path, report: &mut Report) -> Result<()> {
    let opened = crate::desktop::background_process::command("/usr/bin/open")
        .args(["-g", "-a", APP])
        .arg(file)
        .status()?;
    if !opened.success() {
        return Err(eyre!("`open -a TextEdit` failed"));
    }
    let window = wait_for_window(tool)?;
    let id = window.window_id.clone().unwrap_or_default();
    let before = bounds(tool, APP, &id).map_err(|error| eyre!(error))?;
    report.step(
        "open",
        before.is_some(),
        format!("window {id} at {before:?}"),
    );

    let parked = park_now(&window);
    let parked_ok = report.step("park", parked.is_ok(), format!("{parked:?}"));
    std::thread::sleep(Duration::from_millis(500));
    let after = bounds(tool, APP, &id);
    let frame = crate::assistant::computer::window_frame(&window);
    println!("       parked: list-windows bounds {after:?}, on-screen frame {frame:?}");
    if let (Ok(parked), Ok(Some((x, y, _, _)))) = (&parked, &after) {
        let moved = (x - parked.to.0).abs() < 40.0 && (y - parked.to.1).abs() < 40.0;
        report.step(
            "stays parked",
            moved,
            format!("asked for {:?}, macOS put it at ({x}, {y})", parked.to),
        );
    }

    if parked_ok {
        match text_area(tool, &id) {
            Ok((reference, value)) => {
                report.step(
                    "read while parked",
                    value.contains(BEFORE),
                    format!("{reference} reads {value:?}"),
                );
                let set = call(tool, &["set-value", &reference, AFTER]);
                let reread = text_area(tool, &id).map(|(_, value)| value);
                report.step(
                    "set while parked",
                    set.is_ok() && reread.as_deref().is_ok_and(|value| value.contains(AFTER)),
                    format!("set {:?}, now reads {reread:?}", set.err()),
                );
            }
            Err(error) => {
                report.step("read while parked", false, &error);
            }
        }
    }

    let shown = show_now(&window);
    std::thread::sleep(Duration::from_millis(500));
    let back = bounds(tool, APP, &id).map_err(|error| eyre!(error))?;
    let returned = match (before, back) {
        (Some((x0, y0, _, _)), Some((x1, y1, _, _))) => {
            (x0 - x1).abs() < 2.0 && (y0 - y1).abs() < 2.0
        }
        _ => false,
    };
    report.step(
        "show",
        shown.is_ok() && returned,
        format!("{shown:?}, now at {back:?}"),
    );
    Ok(())
}

/// The probe's window, once TextEdit shows it (15 s at most).
fn wait_for_window(tool: &Path) -> Result<TaskWindow> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(data) = call(tool, &["list-windows", "--app", APP]) {
            let found = desktop::windows(&data)
                .into_iter()
                .find(|window| window.title.contains(TITLE));
            if let Some(window) = found {
                return Ok(TaskWindow {
                    app: window.app,
                    window_id: Some(window.id),
                });
            }
        }
        if Instant::now() > deadline {
            return Err(eyre!("TextEdit showed no “{TITLE}” window within 15 s"));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// The window's text area: its ref and value.
fn text_area(tool: &Path, id: &str) -> Result<(String, String), String> {
    let data = call(
        tool,
        &[
            "find",
            "--app",
            APP,
            "--window-id",
            id,
            "--role",
            "textarea",
        ],
    )?;
    let first = data
        .get("matches")
        .and_then(Value::as_array)
        .and_then(|matches| matches.first())
        .ok_or_else(|| format!("no text area; roles present: {}", data["roles_present"]))?;
    let reference = first
        .get("ref_id")
        .and_then(Value::as_str)
        .ok_or("the text area has no ref")?
        .to_owned();
    let value = first
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok((reference, value))
}
