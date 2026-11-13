//! Automation (Apple Events): Hex may control other apps. macOS asks per
//! target app the first time one is scripted, so a task that drives System
//! Events (as Claude Code's `osascript` does to operate apps) used to stop
//! midway for „Hex.app möchte Zugriffsrechte, um „System Events.app“ zu
//! steuern“. Onboarding and the guided repair ask for System Events and
//! Finder up front instead ([`UP_FRONT`]), so most tasks run unattended.
//! Any other app a task scripts asks on its first use: the task's step
//! says so first ([`first_use_notice`]: „macOS asks whether Hex may control
//! Mail …“), so the prompt isn't a surprise and the person knows to allow.
//!
//! Targets Hex itself scripts (they ask on first use, each for its own
//! app): System Events (tasks' `osascript`), the browsers it reads the
//! front tab's address from (`dictation::context`: Safari, Chrome, Arc,
//! Brave, Edge, Vivaldi, Opera, Orion, Helium, Chromium), and the media
//! players it pauses while dictating (Music, Spotify, VLC).
//!
//! ```ts
//! status   AEDeterminePermissionToAutomateTarget(app, ask: false)
//!            0 granted · -1744 not asked yet · -1743 denied · -600 not running
//! request  the app started in the background, then the same call
//!          with ask: true (blocks until the person answers; off the main thread)
//! ```
//!
//! OWNER: onboarding.

use std::sync::atomic::{AtomicU8, Ordering};

/// System Events, the app tasks script to operate other apps.
pub const SYSTEM_EVENTS: &str = "com.apple.systemevents";
/// Messages, which sends the person's iMessages while they're away
/// (`assistant::phone`).
pub const MESSAGES: &str = "com.apple.MobileSMS";
/// Finder, which tasks script for files, folders, and windows.
pub const FINDER: &str = "com.apple.finder";

/// The apps onboarding asks for up front, by bundle ID and process name:
/// nearly every task that scripts the Mac goes through one of them. Every
/// other app asks on its first use.
pub const UP_FRONT: [(&str, &str); 2] = [(SYSTEM_EVENTS, "System Events"), (FINDER, "Finder")];

/// Whether Hex may control the [`UP_FRONT`] apps (or one app).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AutomationState {
    Granted,
    /// macOS hasn't asked yet: a request shows its prompt.
    NotAsked,
    /// The person said no; only System Settings › Automation turns it on.
    Denied,
    /// Not known (not read yet, or the target wasn't running).
    #[default]
    Unknown,
}

impl AutomationState {
    fn code(self) -> u8 {
        match self {
            Self::Granted => 1,
            Self::NotAsked => 2,
            Self::Denied => 3,
            Self::Unknown => 0,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Granted,
            2 => Self::NotAsked,
            3 => Self::Denied,
            _ => Self::Unknown,
        }
    }
}

/// `AEDeterminePermissionToAutomateTarget`'s status as a state.
pub fn state_of(status: i32) -> AutomationState {
    match status {
        0 => AutomationState::Granted,
        // errAEEventWouldRequireUserConsent
        -1744 => AutomationState::NotAsked,
        // errAEEventNotPermitted
        -1743 => AutomationState::Denied,
        // procNotFound and anything else: can't tell.
        _ => AutomationState::Unknown,
    }
}

/// Several apps' states as one: a refusal first (only System Settings
/// fixes it), then one never asked (Allow asks), then one not known.
pub fn combine(states: &[AutomationState]) -> AutomationState {
    [
        AutomationState::Denied,
        AutomationState::NotAsked,
        AutomationState::Unknown,
    ]
    .into_iter()
    .find(|state| states.contains(state))
    .unwrap_or(AutomationState::Granted)
}

static LAST: AtomicU8 = AtomicU8::new(0);

/// The state last read ([`refresh`], [`request`]); `Unknown` before.
pub fn last() -> AutomationState {
    AutomationState::from_code(LAST.load(Ordering::Acquire))
}

fn remember(state: AutomationState) -> AutomationState {
    LAST.store(state.code(), Ordering::Release);
    state
}

/// Reads the state now without asking (starts System Events in the
/// background first, since macOS can only tell for a running app). Blocks
/// for a moment; off the main thread.
pub fn refresh() -> AutomationState {
    remember(determine(false))
}

/// Shows macOS's prompt if it hasn't been answered, and returns the
/// answer. Blocks until the person answers; off the main thread.
pub fn request() -> AutomationState {
    remember(determine(true))
}

/// [`refresh`] on its own thread.
pub fn refresh_async() {
    let _ = std::thread::Builder::new()
        .name("automation-status".into())
        .spawn(|| {
            refresh();
        });
}

/// [`request`] on its own thread; a denial opens System Settings ›
/// Privacy & Security › Automation.
pub fn request_async() {
    let _ = std::thread::Builder::new()
        .name("automation-request".into())
        .spawn(|| {
            if request() == AutomationState::Denied {
                open_settings();
            }
        });
}

/// System Settings › Privacy & Security › Automation.
pub fn open_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation")
            .spawn();
    }
}

/// Whether Hex may control Messages (asked only when the phone channel is
/// turned on); `ask` shows the prompt. Blocks; off the main thread.
pub fn messages(ask: bool) -> AutomationState {
    #[cfg(target_os = "macos")]
    {
        start_app(MESSAGES, "Messages");
        let state = state_of(permission(MESSAGES, ask));
        tracing::info!(?state, ask, "automation for Messages");
        state
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = ask;
        AutomationState::Granted
    }
}

/// Each [`UP_FRONT`] app in turn (one prompt each when asking), as one
/// state.
#[cfg(target_os = "macos")]
fn determine(ask: bool) -> AutomationState {
    let states: Vec<AutomationState> = UP_FRONT
        .iter()
        .map(|(bundle_id, process)| {
            start_app(bundle_id, process);
            let status = permission(bundle_id, ask);
            let state = state_of(status);
            tracing::info!(status, ?state, ask, bundle_id, "automation");
            state
        })
        .collect();
    combine(&states)
}

#[cfg(not(target_os = "macos"))]
fn determine(_ask: bool) -> AutomationState {
    AutomationState::Granted
}

/// The apps an AppleScript or JXA snippet scripts, by name, each once:
/// `tell application "Mail"`, `tell app "Mail"`, `Application("Mail")`,
/// `Application('Mail')`. Case doesn't matter; an `application id "…"`
/// names no app here.
pub fn scripted_apps(script: &str) -> Vec<String> {
    let lower = script.to_ascii_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let name = name.trim();
        if !name.is_empty()
            && name.len() <= 80
            && !found.iter().any(|seen| seen.eq_ignore_ascii_case(name))
        {
            found.push(name.to_owned());
        }
    };
    for opener in ["tell application ", "tell app ", "application("] {
        for (start, _) in lower.match_indices(opener) {
            // Only a whole word: "Application(" in JXA, not "myapp(".
            let before = lower[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            // Byte offsets match: ASCII lowercasing keeps every length.
            let rest = script.get(start + opener.len()..).unwrap_or_default();
            let rest = rest.trim_start_matches([' ', '\\']);
            let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
                continue;
            };
            let body = &rest[1..];
            let end = body.find([quote, '\\', '\n']);
            if let Some(end) = end {
                push(&body[..end]);
            }
        }
    }
    found
}

/// What a task's tool call scripts, from its input (a command's JSON, or
/// plain text): every string in it that runs AppleScript or JXA.
pub fn scripted_by_tool(input: &str) -> Vec<String> {
    fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) => out.push(text.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
            serde_json::Value::Object(map) => map.values().for_each(|item| strings(item, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    match serde_json::from_str::<serde_json::Value>(input) {
        Ok(value) => strings(&value, &mut texts),
        Err(_) => texts.push(input.replace("\\\"", "\"")),
    }
    let mut apps: Vec<String> = Vec::new();
    for text in texts {
        let lower = text.to_lowercase();
        if !(lower.contains("osascript")
            || lower.contains("applescript")
            || lower.contains("tell app"))
        {
            continue;
        }
        for app in scripted_apps(&text) {
            if !apps.iter().any(|seen| seen.eq_ignore_ascii_case(&app)) {
                apps.push(app);
            }
        }
    }
    apps
}

/// The step a task shows before its script reaches an app macOS hasn't
/// asked about yet (or was refused), so the prompt that stops it is
/// expected: once per app while Hex runs. `None` when nothing in `input`
/// scripts such an app, for the [`UP_FRONT`] apps (onboarding asks for
/// those), and for an app this Mac doesn't have.
pub fn first_use_notice(input: &str) -> Option<String> {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static NOTICED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    if cfg!(test) || !cfg!(target_os = "macos") {
        return None;
    }
    let apps = scripted_by_tool(input);
    if apps.is_empty() {
        return None;
    }
    let mut noticed = NOTICED.lock().unwrap_or_else(|e| e.into_inner());
    let up_front: HashSet<&str> = UP_FRONT.iter().map(|(_, name)| *name).collect();
    for app in apps {
        if up_front.iter().any(|name| name.eq_ignore_ascii_case(&app))
            || noticed.iter().any(|seen| seen.eq_ignore_ascii_case(&app))
        {
            continue;
        }
        noticed.push(app.clone());
        let Some(bundle_id) = bundle_id_of(&app) else {
            continue;
        };
        if bundle_id == SYSTEM_EVENTS || bundle_id == FINDER {
            continue;
        }
        let state = state_of(app_permission(&bundle_id));
        tracing::info!(app, bundle_id, ?state, "a task scripts an app");
        return notice_for(&app, state);
    }
    None
}

/// The words for `app` in `state`; nothing once it is allowed.
pub fn notice_for(app: &str, state: AutomationState) -> Option<String> {
    use crate::desktop::i18n::tr_fill;
    match state {
        AutomationState::Granted => None,
        AutomationState::NotAsked => Some(tr_fill(
            "macOS asks whether Hex may control {}: choose Allow so the task can go on",
            app,
        )),
        // Not running, so macOS couldn't tell: it may ask as the app opens.
        AutomationState::Unknown => Some(tr_fill(
            "If macOS asks whether Hex may control {}, choose Allow so the task can go on",
            app,
        )),
        AutomationState::Denied => Some(tr_fill(
            "Hex may not control {}: turn it on in System Settings › Privacy & Security › Automation",
            app,
        )),
    }
}

#[cfg(target_os = "macos")]
fn app_permission(bundle_id: &str) -> i32 {
    permission(bundle_id, false)
}

#[cfg(not(target_os = "macos"))]
fn app_permission(_: &str) -> i32 {
    0
}

/// The bundle ID of the app called `name` in the usual app folders (its
/// `Name.app`), read from its Info.plist.
fn bundle_id_of(name: &str) -> Option<String> {
    if name.contains('/') || name.contains("..") {
        return None;
    }
    let home = crate::app_paths::home_dir();
    let folders = [
        Some(std::path::PathBuf::from("/Applications")),
        Some(std::path::PathBuf::from("/Applications/Utilities")),
        Some(std::path::PathBuf::from("/System/Applications")),
        Some(std::path::PathBuf::from("/System/Applications/Utilities")),
        Some(std::path::PathBuf::from("/System/Library/CoreServices")),
        home.map(|home| home.join("Applications")),
    ];
    let bundle = folders
        .into_iter()
        .flatten()
        .map(|folder| folder.join(format!("{name}.app")))
        .find(|path| path.is_dir())?;
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist"))
        .output()
        .ok()?;
    let id = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !id.is_empty()).then_some(id)
}

/// Starts `bundle_id` (process `process`) in the background when it isn't
/// running, waiting up to 3 s.
#[cfg(target_os = "macos")]
fn start_app(bundle_id: &str, process: &str) {
    let running = || {
        std::process::Command::new("/usr/bin/pgrep")
            .args(["-x", process])
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    if running() {
        return;
    }
    let _ = std::process::Command::new("/usr/bin/open")
        .args(["-g", "-j", "-b", bundle_id])
        .status();
    for _ in 0..30 {
        if running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(
        type_code: u32,
        data: *const std::ffi::c_void,
        size: isize,
        result: *mut AEDesc,
    ) -> i16;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AEDesc,
        event_class: u32,
        event_id: u32,
        ask_user_if_needed: u8,
    ) -> i32;
}

#[cfg(target_os = "macos")]
const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

/// `AEDeterminePermissionToAutomateTarget` for `bundle_id`, any event.
#[cfg(target_os = "macos")]
fn permission(bundle_id: &str, ask: bool) -> i32 {
    let mut desc = AEDesc {
        descriptor_type: 0,
        data_handle: std::ptr::null_mut(),
    };
    // SAFETY: the bundle id's bytes outlive the call; AECreateDesc copies
    // them into the descriptor it fills, which is disposed below.
    let created = unsafe {
        AECreateDesc(
            four_cc(b"bund"),
            bundle_id.as_ptr().cast(),
            bundle_id.len() as isize,
            &mut desc,
        )
    };
    if created != 0 {
        return i32::from(created);
    }
    // SAFETY: `desc` is a valid descriptor made above.
    let status = unsafe {
        AEDeterminePermissionToAutomateTarget(
            &desc,
            four_cc(b"****"),
            four_cc(b"****"),
            u8::from(ask),
        )
    };
    // SAFETY: disposes the descriptor made above, once.
    unsafe {
        AEDisposeDesc(&mut desc);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_statuses_read_as_states() {
        assert_eq!(state_of(0), AutomationState::Granted);
        assert_eq!(state_of(-1744), AutomationState::NotAsked);
        assert_eq!(state_of(-1743), AutomationState::Denied);
        assert_eq!(state_of(-600), AutomationState::Unknown);
        assert_eq!(state_of(-50), AutomationState::Unknown);
    }

    #[test]
    fn states_survive_their_code() {
        for state in [
            AutomationState::Granted,
            AutomationState::NotAsked,
            AutomationState::Denied,
            AutomationState::Unknown,
        ] {
            assert_eq!(AutomationState::from_code(state.code()), state);
        }
    }

    #[test]
    fn the_up_front_apps_read_as_their_least_ready() {
        use AutomationState::*;
        assert_eq!(combine(&[Granted, Granted]), Granted);
        assert_eq!(combine(&[Granted, NotAsked]), NotAsked);
        assert_eq!(combine(&[NotAsked, Denied]), Denied);
        assert_eq!(combine(&[Unknown, Granted]), Unknown);
        assert_eq!(combine(&[Unknown, NotAsked]), NotAsked);
        assert_eq!(UP_FRONT.map(|(id, _)| id), [SYSTEM_EVENTS, FINDER]);
    }

    #[test]
    fn scripts_name_the_apps_they_control() {
        assert_eq!(
            scripted_apps(r#"tell application "Mail" to get name of every account"#),
            ["Mail"]
        );
        assert_eq!(
            scripted_apps("tell app 'Microsoft Excel'\nactivate\nend tell"),
            ["Microsoft Excel"]
        );
        assert_eq!(
            scripted_apps(r#"Application("Numbers").documents(); Application('Mail')"#),
            ["Numbers", "Mail"]
        );
        // Each once, whatever its case; nothing from look-alikes.
        assert_eq!(
            scripted_apps(r#"TELL APPLICATION "Notes" … tell application "notes""#),
            ["Notes"]
        );
        assert!(scripted_apps("currentApplication() myApplication('x')").is_empty());
        assert!(scripted_apps(r#"tell application id "com.apple.mail""#).is_empty());
    }

    #[test]
    fn a_tool_call_names_what_its_osascript_scripts() {
        let bash = r#"{"command":"osascript -e 'tell application \"Calendar\" to activate'"}"#;
        assert_eq!(scripted_by_tool(bash), ["Calendar"]);
        let escaped = r#"{"command":"osascript -e \"tell application \\\"Mail\\\" to quit\""}"#;
        assert_eq!(scripted_by_tool(escaped), ["Mail"]);
        // Plain text a backend cut short still reads.
        let cut = r#"{"command":"osascript -e 'tell application \"Keynote\" to"#;
        assert_eq!(scripted_by_tool(cut), ["Keynote"]);
        // Text that merely names an app isn't a script.
        assert!(
            scripted_by_tool(r#"{"command":"open -a Mail; echo Application(\"Mail\")"}"#)
                .is_empty()
        );
    }

    #[test]
    fn the_notice_says_what_macos_asks_and_nothing_once_allowed() {
        assert_eq!(notice_for("Mail", AutomationState::Granted), None);
        let asks = notice_for("Mail", AutomationState::NotAsked).unwrap();
        assert!(asks.contains("Mail") && asks.contains("Allow"), "{asks}");
        let refused = notice_for("Mail", AutomationState::Denied).unwrap();
        assert!(refused.contains("Automation"), "{refused}");
        assert!(
            notice_for("Mail", AutomationState::Unknown)
                .unwrap()
                .starts_with("If macOS asks")
        );
    }
}
