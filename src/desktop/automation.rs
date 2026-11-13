//! Automation (Apple Events): Hex may control other apps. macOS asks per
//! target app the first time one is scripted, so a task that drives System
//! Events (as Claude Code's `osascript` does to operate apps) used to stop
//! midway for „Hex.app möchte Zugriffsrechte, um „System Events.app“ zu
//! steuern“. Onboarding and the guided repair ask for System Events up
//! front instead, so tasks run unattended.
//!
//! Targets Hex itself scripts (they ask on first use, each for its own
//! app): System Events (tasks' `osascript`), the browsers it reads the
//! front tab's address from (`dictation::context`: Safari, Chrome, Arc,
//! Brave, Edge, Vivaldi, Opera, Orion, Helium, Chromium), and the media
//! players it pauses while dictating (Music, Spotify, VLC).
//!
//! ```ts
//! status   AEDeterminePermissionToAutomateTarget(System Events, ask: false)
//!            0 granted · -1744 not asked yet · -1743 denied · -600 not running
//! request  System Events started in the background, then the same call
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

/// Whether Hex may control System Events.
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

#[cfg(target_os = "macos")]
fn determine(ask: bool) -> AutomationState {
    start_system_events();
    let status = permission(SYSTEM_EVENTS, ask);
    let state = state_of(status);
    tracing::info!(status, ?state, ask, "automation for System Events");
    state
}

#[cfg(not(target_os = "macos"))]
fn determine(_ask: bool) -> AutomationState {
    AutomationState::Granted
}

/// Starts System Events in the background when it isn't running (macOS
/// can only tell, and ask, for a running app), waiting up to 3 s.
#[cfg(target_os = "macos")]
fn start_system_events() {
    start_app(SYSTEM_EVENTS, "System Events");
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
}
