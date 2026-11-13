//! An app's window onto this screen when it has none here.
//!
//! macOS has no public way to move another app's window between desktops
//! (Spaces), and Accessibility doesn't list a window on another desktop or
//! in full screen (its own Space), so Cua Driver can neither read nor
//! operate it ("nicht lesbar, Zeitfehler"). What works, cheapest first,
//! for any app (no list of apps: what an app is comes from the app itself,
//! its bundle's `Info.plist` and what its windows say):
//!
//! 1. The app is hidden (⌘H): shown again, without activating it.
//! 2. Its window is in full screen and Accessibility reaches it: full
//!    screen ends (`AXFullScreen` off), so it is an ordinary window here.
//! 3. Its window is minimized: restored (onto this desktop).
//! 4. The app is scriptable (`NSAppleScriptEnabled`): it is asked for a new
//!    window (`make new window`, `make new document`, …); new windows open
//!    on the desktop the person is on.
//! 5. The app isn't document-based (no `NSDocumentClass` among its document
//!    types: messengers, players, browsers, Electron apps) and none of its
//!    windows says it has unsaved changes: it is quit quietly and reopened
//!    in the background (`open -g`, without restoring its last window
//!    state, so it doesn't come back in full screen or on that desktop);
//!    the guard parks the new window off the screen at once.
//! 6. Else it is said clearly, so the model doesn't loop or ask the person
//!    to move windows.
//!
//! [`plan`] decides (pure, tested); the guard carries a step out and looks
//! again, at most once per step, within a few seconds.

/// What an app's bundle says about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traits {
    /// It keeps documents (`NSDocumentClass` in `CFBundleDocumentTypes`):
    /// quitting could lose unsaved work, so it is never quit.
    pub document_based: bool,
    /// It takes AppleScript (`NSAppleScriptEnabled`).
    pub scriptable: bool,
}

/// [`Traits`] from an app bundle's `Info.plist`, as JSON (`plutil`).
pub fn traits_of(info: &serde_json::Value) -> Traits {
    let document_based = info
        .get("CFBundleDocumentTypes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|types| {
            types.iter().any(|kind| {
                kind.get("NSDocumentClass")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|class| !class.trim().is_empty())
            })
        });
    let scriptable = match info.get("NSAppleScriptEnabled") {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::String(text)) => text.eq_ignore_ascii_case("yes") || text == "true",
        _ => false,
    } || info.get("OSAScriptingDefinition").is_some();
    Traits {
        document_based,
        scriptable,
    }
}

/// [`Traits`] of the app bundle at `path` (read with `plutil`, a few
/// milliseconds); nothing known when it can't be read.
pub fn traits(path: &std::path::Path) -> Traits {
    let info = path.join("Contents/Info.plist");
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&info)
        .output();
    match output {
        Ok(output) if output.status.success() => serde_json::from_slice(&output.stdout)
            .map(|info| traits_of(&info))
            .unwrap_or_default(),
        _ => Traits::default(),
    }
}

/// The ways a scriptable app is asked for a new window, tried in order
/// (the first its dictionary knows wins).
pub const NEW_WINDOW_COMMANDS: &[&str] = &[
    "make new window",
    "make new document",
    "make new Finder window",
    "make new message viewer",
];

/// The AppleScript that asks app `bundle_id` for a new window, trying each
/// of [`NEW_WINDOW_COMMANDS`].
pub fn new_window_script(bundle_id: &str) -> String {
    let id = bundle_id.replace('\\', "").replace('"', "");
    let tries: String = NEW_WINDOW_COMMANDS
        .iter()
        .map(|command| format!("\ttry\n\t\t{command}\n\t\treturn \"ok\"\n\tend try\n"))
        .collect();
    format!("tell application id \"{id}\"\n{tries}end tell\nreturn \"none\"")
}

/// One window of the app as the guard sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowState {
    pub id: String,
    /// On this desktop's screen now.
    pub on_screen: bool,
    /// Accessibility lists it, so it can be reached from here.
    pub listed: bool,
    pub minimized: bool,
    pub full_screen: bool,
    /// It says it has unsaved changes.
    pub edited: bool,
}

/// An app whose window the guard wants on this screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    pub name: String,
    pub bundle_id: Option<String>,
    pub traits: Traits,
    /// Hidden (⌘H).
    pub hidden: bool,
    pub windows: Vec<WindowState>,
}

/// What to do next.
#[derive(Clone, Debug, PartialEq)]
pub enum Recovery {
    /// A window is on this screen: take it.
    Ready(String),
    /// Show the hidden app (without activating it).
    Unhide,
    /// End the window's full screen.
    LeaveFullScreen(String),
    /// Restore the minimized window.
    Restore(String),
    /// Ask the app for a new window by script.
    NewWindow(String),
    /// Quit quietly and reopen in the background.
    Relaunch,
    /// Nothing more to try; why, for the model.
    Report(String),
}

impl Recovery {
    /// Its kind, to remember what was tried.
    pub fn kind(&self) -> Tried {
        match self {
            Self::Ready(_) | Self::Report(_) => Tried::Nothing,
            Self::Unhide => Tried::Unhide,
            Self::LeaveFullScreen(_) => Tried::LeaveFullScreen,
            Self::Restore(_) => Tried::Restore,
            Self::NewWindow(_) => Tried::NewWindow,
            Self::Relaunch => Tried::Relaunch,
        }
    }
}

/// A step already tried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tried {
    Nothing,
    Unhide,
    LeaveFullScreen,
    Restore,
    NewWindow,
    Relaunch,
}

/// Whether the app may be quit and reopened without asking: it isn't
/// document-based and no window of it says it has unsaved changes.
pub fn quiet_relaunch(state: &AppState) -> bool {
    !state.traits.document_based && !state.windows.iter().any(|window| window.edited)
}

/// The next step for `state`, given the steps `tried` (each at most once).
pub fn plan(state: &AppState, tried: &[Tried]) -> Recovery {
    let not_tried = |step: Tried| !tried.contains(&step);
    if let Some(window) = state
        .windows
        .iter()
        .find(|window| window.on_screen && !window.minimized && !window.full_screen)
    {
        return Recovery::Ready(window.id.clone());
    }
    if state.hidden && not_tried(Tried::Unhide) {
        return Recovery::Unhide;
    }
    if not_tried(Tried::LeaveFullScreen)
        && let Some(window) = state
            .windows
            .iter()
            .find(|window| window.listed && window.full_screen)
    {
        return Recovery::LeaveFullScreen(window.id.clone());
    }
    // A relaunched app may bring its window back minimized: restoring is
    // tried again after a relaunch.
    let since_relaunch = tried
        .iter()
        .rposition(|step| *step == Tried::Relaunch)
        .map_or(tried, |at| &tried[at + 1..]);
    if !since_relaunch.contains(&Tried::Restore)
        && let Some(window) = state.windows.iter().find(|window| window.minimized)
    {
        return Recovery::Restore(window.id.clone());
    }
    if not_tried(Tried::NewWindow)
        && state.traits.scriptable
        && let Some(id) = &state.bundle_id
    {
        return Recovery::NewWindow(new_window_script(id));
    }
    let edited = state.windows.iter().any(|window| window.edited);
    if quiet_relaunch(state) && not_tried(Tried::Relaunch) {
        return Recovery::Relaunch;
    }
    let name = &state.name;
    Recovery::Report(if edited {
        format!(
            "{name}'s window is on another desktop (or in full screen) and has unsaved changes, \
             so Agent-Desk didn't quit it to bring it here. {SWITCH_METHOD}"
        )
    } else if tried.contains(&Tried::Relaunch) || tried.contains(&Tried::NewWindow) {
        format!(
            "{name} was asked for a window on this desktop but none came within a few seconds. \
             Don't retry the snapshot. {SWITCH_METHOD}"
        )
    } else {
        format!(
            "{name}'s window is on another desktop (or in full screen), where Agent-Desk can neither \
             read nor operate it, and {name} keeps documents, so it isn't quit. {SWITCH_METHOD}"
        )
    })
}

/// What an unreachable window means: another way, never the person.
pub const SWITCH_METHOD: &str = "Switch method instead of waiting for the window: \
hex_app_capabilities shows what the app offers; script it with hex_app_action (intent script \
reads and changes its data without any window, on any desktop), or use a URL scheme it declares \
(intent open_url), or read the window with hex_read_window. Never ask the person to bring the \
window over or to try again later.";

/// What a finished recovery says in the tool's answer.
pub fn recovered_note(step: Tried) -> &'static str {
    match step {
        Tried::Unhide => {
            "The app was hidden; Agent-Desk showed it again without bringing it in front. Its window is \
             yours, parked out of the person's way: act in it by its id."
        }
        Tried::LeaveFullScreen => {
            "The app's window was in full screen (a desktop of its own, unreadable from here); \
             Agent-Desk ended its full screen and parked it out of the person's way. It is yours: act \
             in it by its id."
        }
        Tried::Restore => {
            "The app's window was minimized; Agent-Desk restored it, parked out of the person's way. It \
             is yours: act in it by its id."
        }
        Tried::NewWindow => {
            "The app's windows were on another desktop; Agent-Desk asked it for a new window on this \
             one, parked out of the person's way. It is yours: act in it by its id."
        }
        Tried::Relaunch => {
            "The app's only window was on another desktop (or in full screen); Agent-Desk quit it \
             quietly and reopened it in the background, so its window is on this desktop now, \
             parked out of the person's way. It is yours: act in it by its id."
        }
        Tried::Nothing => "The app's window is yours: act in it by its id.",
    }
}

/// Joins the guard's view of an app's windows with what Accessibility says
/// (`ax`: window number, minimized, full screen, edited for each window it
/// lists).
pub fn app_state(
    name: &str,
    bundle_id: Option<&str>,
    traits: Traits,
    hidden: bool,
    on_screen: &[String],
    off_screen: &[String],
    ax: &[super::ax::AxWindow],
) -> AppState {
    let of = |id: &str| {
        let number = crate::assistant::computer::window_number(id);
        ax.iter().find(|window| Some(window.number) == number)
    };
    let window = |id: &String, shown: bool| {
        let listed = of(id);
        WindowState {
            id: id.clone(),
            on_screen: shown,
            listed: listed.is_some(),
            minimized: listed.is_some_and(|window| window.minimized),
            full_screen: listed.is_some_and(|window| window.full_screen),
            edited: listed.is_some_and(|window| window.edited),
        }
    };
    let mut windows: Vec<WindowState> = on_screen.iter().map(|id| window(id, true)).collect();
    windows.extend(off_screen.iter().map(|id| window(id, false)));
    // Accessibility's windows the window server didn't name for this app:
    // their full screen and edits count.
    for listed in ax {
        let id = format!("w-{}", listed.number);
        if !windows.iter().any(|window| window.id == id) {
            windows.push(WindowState {
                id,
                on_screen: false,
                listed: true,
                minimized: listed.minimized,
                full_screen: listed.full_screen,
                edited: listed.edited,
            });
        }
    }
    AppState {
        name: name.to_owned(),
        bundle_id: bundle_id.map(str::to_owned),
        traits,
        hidden,
        windows,
    }
}

// ── The steps on this Mac ──────────────────────────────────────────────

/// Shows a hidden app without activating it; whether it was asked.
pub fn unhide(pid: i32) -> bool {
    super::ax::unhide(pid)
}

/// Asks for a new window with `script` (AppleScript); whether one came.
pub fn new_window(script: &str) -> bool {
    run_osascript(script).is_ok_and(|answer| answer == "ok")
}

/// Quits the app (`pids`) politely, as ⌘Q does, waits until it has ended
/// (at most `wait`), and reopens it in the background (`open -g`) without
/// its saved window state (so not in full screen, not on that desktop).
/// Whether it was reopened.
pub fn relaunch(
    pids: &[i64],
    bundle_id: Option<&str>,
    path: &std::path::Path,
    wait: std::time::Duration,
) -> bool {
    let started = std::time::Instant::now();
    let pids: Vec<i32> = pids
        .iter()
        .filter_map(|pid| i32::try_from(*pid).ok())
        .collect();
    for pid in &pids {
        super::ax::terminate(*pid);
    }
    while pids
        .iter()
        .any(|pid| crate::assistant::guard::process_alive(*pid as u32))
    {
        if started.elapsed() >= wait {
            tracing::warn!(?pids, "the app didn't quit in time; not reopening it");
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut open = std::process::Command::new("/usr/bin/open");
    open.arg("-g");
    match bundle_id {
        Some(id) => open.args(["-b", id]),
        None => open.arg(path),
    };
    open.args(["--args", "-ApplePersistenceIgnoreState", "YES"]);
    let opened = open
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    tracing::info!(
        opened,
        took_ms = started.elapsed().as_millis() as u64,
        "quit an app quietly and reopened it in the background"
    );
    opened
}

/// Runs AppleScript `script`; its output, or why not.
pub fn run_osascript(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .map_err(|error| format!("osascript: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::super::ax::AxWindow;
    use super::*;
    use serde_json::json;

    fn app(traits: Traits, windows: Vec<WindowState>) -> AppState {
        AppState {
            name: "\u{200e}WhatsApp".into(),
            bundle_id: Some("net.whatsapp.WhatsApp".into()),
            traits,
            hidden: false,
            windows,
        }
    }

    fn off(id: &str) -> WindowState {
        WindowState {
            id: id.into(),
            ..Default::default()
        }
    }

    const PLAIN: Traits = Traits {
        document_based: false,
        scriptable: false,
    };

    #[test]
    fn what_an_app_is_comes_from_its_bundle_not_a_list() {
        // TextEdit: NSDocument-based and scriptable.
        let textedit = json!({
            "CFBundleDocumentTypes": [{"CFBundleTypeName": "RTF", "NSDocumentClass": "Document"}],
            "NSAppleScriptEnabled": "YES",
        });
        assert_eq!(
            traits_of(&textedit),
            Traits {
                document_based: true,
                scriptable: true
            }
        );
        // A browser opens files but keeps no documents; scriptable by sdef.
        let chrome = json!({
            "CFBundleDocumentTypes": [{"CFBundleTypeName": "HTML", "LSItemContentTypes": ["public.html"]}],
            "OSAScriptingDefinition": "scripting.sdef",
        });
        assert_eq!(
            traits_of(&chrome),
            Traits {
                document_based: false,
                scriptable: true
            }
        );
        // A messenger: neither.
        assert_eq!(traits_of(&json!({"CFBundleName": "WhatsApp"})), PLAIN);
    }

    #[test]
    fn a_window_on_screen_is_taken_as_it_is() {
        let mut shown = off("w-2");
        shown.on_screen = true;
        assert_eq!(
            plan(&app(PLAIN, vec![off("w-1"), shown]), &[]),
            Recovery::Ready("w-2".into())
        );
    }

    #[test]
    fn hidden_then_full_screen_then_minimized_then_relaunch_then_report() {
        let mut state = app(PLAIN, vec![off("w-1")]);
        state.hidden = true;
        assert_eq!(plan(&state, &[]), Recovery::Unhide);
        state.windows[0].listed = true;
        state.windows[0].full_screen = true;
        assert_eq!(
            plan(&state, &[Tried::Unhide]),
            Recovery::LeaveFullScreen("w-1".into())
        );
        state.windows[0].minimized = true;
        let tried = [Tried::Unhide, Tried::LeaveFullScreen];
        assert_eq!(plan(&state, &tried), Recovery::Restore("w-1".into()));
        // Not scriptable, not document-based: quit and reopen.
        let tried = [Tried::Unhide, Tried::LeaveFullScreen, Tried::Restore];
        assert_eq!(plan(&state, &tried), Recovery::Relaunch);
        let tried = [
            Tried::Unhide,
            Tried::LeaveFullScreen,
            Tried::Restore,
            Tried::Relaunch,
        ];
        // It came back minimized: restored once more.
        assert_eq!(plan(&state, &tried), Recovery::Restore("w-1".into()));
        let tried = [
            Tried::Unhide,
            Tried::LeaveFullScreen,
            Tried::Restore,
            Tried::Relaunch,
            Tried::Restore,
        ];
        let Recovery::Report(why) = plan(&state, &tried) else {
            panic!("expected a report");
        };
        assert!(why.contains("none came within a few seconds"), "{why}");
        assert!(why.contains("hex_app_capabilities") && why.contains("Never ask the person"));
    }

    #[test]
    fn a_window_that_comes_back_minimized_after_a_relaunch_is_restored() {
        let mut window = off("w-7");
        window.listed = true;
        window.minimized = true;
        let state = app(PLAIN, vec![window]);
        // Restored once (it failed: the app didn't answer), relaunched, and
        // its new window is minimized again: restore it, don't report.
        let tried = [Tried::Restore, Tried::Relaunch];
        assert_eq!(plan(&state, &tried), Recovery::Restore("w-7".into()));
        let tried = [Tried::Restore, Tried::Relaunch, Tried::Restore];
        assert!(matches!(plan(&state, &tried), Recovery::Report(_)));
    }

    #[test]
    fn a_full_screen_window_on_screen_is_not_ready() {
        // In full screen it is a desktop of its own: Accessibility can't
        // read it from the person's desktop.
        let mut window = off("w-1");
        window.on_screen = true;
        window.full_screen = true;
        window.listed = true;
        assert_eq!(
            plan(&app(PLAIN, vec![window]), &[]),
            Recovery::LeaveFullScreen("w-1".into())
        );
        // Out of reach (not listed): quit and reopen without its state.
        assert_eq!(plan(&app(PLAIN, vec![off("w-1")]), &[]), Recovery::Relaunch);
    }

    #[test]
    fn unsaved_changes_or_documents_block_a_quiet_relaunch() {
        let mut window = off("w-1");
        window.edited = true;
        let Recovery::Report(why) = plan(&app(PLAIN, vec![window]), &[]) else {
            panic!("expected a report");
        };
        assert!(why.contains("unsaved changes"), "{why}");
        let documents = Traits {
            document_based: true,
            scriptable: false,
        };
        let Recovery::Report(why) = plan(&app(documents, vec![off("w-1")]), &[]) else {
            panic!("expected a report");
        };
        assert!(why.contains("keeps documents"), "{why}");
    }

    #[test]
    fn a_scriptable_app_is_asked_for_a_new_window_first() {
        let browser = Traits {
            document_based: false,
            scriptable: true,
        };
        let mut state = app(browser, vec![off("w-5")]);
        state.bundle_id = Some("com.apple.Safari".into());
        let Recovery::NewWindow(script) = plan(&state, &[]) else {
            panic!("expected a new window");
        };
        assert!(script.starts_with("tell application id \"com.apple.Safari\""));
        assert!(script.contains("make new window") && script.contains("make new document"));
        // Then, as it keeps no documents, a quiet relaunch.
        assert_eq!(plan(&state, &[Tried::NewWindow]), Recovery::Relaunch);
        // A document app gets the new window, never a quit.
        let documents = Traits {
            document_based: true,
            scriptable: true,
        };
        let state = app(documents, vec![off("w-6")]);
        assert!(matches!(plan(&state, &[]), Recovery::NewWindow(_)));
        assert!(matches!(
            plan(&state, &[Tried::NewWindow]),
            Recovery::Report(_)
        ));
    }

    #[test]
    fn accessibility_tells_minimized_full_screen_and_edited_windows_apart() {
        let ax = [
            AxWindow {
                number: 1,
                minimized: true,
                full_screen: false,
                edited: false,
            },
            AxWindow {
                number: 3,
                minimized: false,
                full_screen: true,
                edited: false,
            },
            AxWindow {
                number: 9,
                minimized: false,
                full_screen: false,
                edited: true,
            },
        ];
        let state = app_state(
            "TextEdit",
            Some("com.apple.TextEdit"),
            Traits::default(),
            false,
            &["w-3".into()],
            &["w-1".into(), "w-2".into()],
            &ax,
        );
        assert_eq!(state.windows.len(), 4);
        assert!(state.windows[0].full_screen && state.windows[0].on_screen);
        assert!(state.windows[1].minimized && state.windows[1].listed);
        // w-2 isn't listed: on another desktop.
        assert!(!state.windows[2].listed && !state.windows[2].minimized);
        // w-9, unnamed by the window server, still counts as edited.
        assert!(state.windows[3].edited);
    }
}
