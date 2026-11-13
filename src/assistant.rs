//! The assistant (on by default, `assistant.enabled`): a second, "Action"
//! shortcut. Hold it and say anything; Agent-Desk decides whether it is a quick
//! turn (the Here lane: about the window in front, answered beside the HUD
//! and read aloud) or a job (the Background lane: its own windows and
//! hidden browsers, beside the others, its dot on the orb). Double-tap shows
//! the job list. The Assistant pane is one stream of every quick turn and
//! job and takes typed messages; Assistant settings hold its settings and
//! shortcuts.
//!
//! Agent-Desk does the voice, the overlays, and the window. The thinking is done by
//! an agent already on this computer (Claude Code, Codex, OpenCode) or the
//! Claude API; every desktop action goes through the guard, Agent-Desk's MCP server
//! in front of Cua Driver (`driver`). `docs/features/assistant.md` and AGENTS.md's
//! "Assistant architecture" have the whole picture.
//!
//! ```ts
//! Action shortcut (assistant::shortcut) ─┐                          ┌─ backend (provider): claude -p | codex exec | opencode | API
//!                                        ├─ service::submit ─ agent ┤     └─ guard (MCP) ─ Cua Driver; browser; tools; knowledge
//! Assistant pane (rune_ui::chats) ───────┘   lanes, queue      │    └─ speech: reads quick answers aloud
//!                                                              └─ store: conversations on disk
//! service::subscribe ─ Event ─ Here bubble, orb and job list, screen frame, window outlines, the Assistant pane
//! ```
//!
//! Everything here is inert while `assistant.enabled` is off.

// Parts only some OSes and the CLI call (`main.rs` allows dead code off
// macOS).
#![cfg_attr(target_os = "macos", allow(dead_code))]
#![allow(unused_imports)]

pub mod actions;
pub mod agent;
pub mod audit;
pub mod autonomy;
pub mod backends;
pub mod browser;
pub mod capacity;
pub mod cli;
pub mod computer;
pub mod credentials;
pub mod driver;
pub mod guard;
pub mod input;
pub mod interaction;
pub mod knowledge;
pub mod lines;
pub mod media;
pub mod memory;
pub mod notify;
pub mod palette;
pub mod phone;
pub mod preferences;
pub mod procedures;
pub mod projects;
pub mod provider;
pub mod route;
pub mod schedules;
pub mod secrets;
pub mod service;
pub mod shortcut;
pub mod space;
pub mod speech;
pub mod store;
pub mod tools;
pub mod types;
pub mod view;

pub use service::{
    Inbox, busy, cancel, cancel_all, conversation, conversations, defer_question, delete, focus,
    hand_back, inbox, prewarm, reading_aloud, resume_interrupted, set_focus, set_listening,
    set_spoken_announcements, spoken_announcements, stop_all, submit, subscribe, take_over, tasks,
};
pub use types::*;

use crate::app_settings::AppSettings;

/// Whether the assistant may run: this OS has it (macOS, Linux, and
/// Windows) and its switch is on.
pub fn enabled(settings: &AppSettings) -> bool {
    available() && settings.assistant.enabled
}

/// Whether this OS has the assistant: every desktop Agent-Desk runs on.
pub const fn available() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
}

/// What essential piece of setup is missing now, as `desktop::setup_gaps`
/// says it (English); `None` when nothing is. The window's launch check sets
/// it; a hold of the Action shortcut then says it and opens the guide
/// instead of asking.
static SETUP_GAP: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

pub fn set_setup_gap(gap: Option<&'static str>) {
    *SETUP_GAP.lock().unwrap_or_else(|error| error.into_inner()) = gap;
}

pub fn setup_gap() -> Option<&'static str> {
    *SETUP_GAP.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether the assistant may answer: enabled, and its setup finished (or
/// its last page, where the gestures are tried, showing). Until then a hold
/// of the Action shortcut opens the setup instead of asking.
pub fn ready(settings: &AppSettings) -> bool {
    enabled(settings) && (settings.assistant.setup_done || trying())
}

/// Set while the setup's last page shows with its checks passed: the
/// gestures it asks the person to try must really work there, though
/// `setup_done` waits for Finish.
static TRYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_trying(on: bool) {
    TRYING.store(on, std::sync::atomic::Ordering::Release);
}

/// Whether the setup's gesture page lets the assistant answer now.
pub fn trying() -> bool {
    TRYING.load(std::sync::atomic::Ordering::Acquire)
}

/// Whether a task gets full control of the Mac (every window, the one in
/// front too, and any app, in either lane): the setting allows it and the
/// request doesn't ask to stay in the background („arbeite nur im
/// Hintergrund“, „lass meine Fenster in Ruhe“, "only in the background").
pub fn full_control(allowed: bool, request: &str) -> bool {
    allowed && !route::asks_background_only(request)
}

#[cfg(test)]
mod full_control_tests {
    #[test]
    fn full_control_unless_asked_to_stay_in_the_background() {
        assert!(super::full_control(true, "Übernimm das Citrix-Fenster"));
        assert!(!super::full_control(
            true,
            "Recherchiere das, aber arbeite nur im Hintergrund"
        ));
        assert!(!super::full_control(
            true,
            "Lass meine Fenster in Ruhe und sortier die Downloads"
        ));
        assert!(!super::full_control(true, "Do it only in the background"));
        assert!(!super::full_control(false, "Übernimm das Citrix-Fenster"));
    }
}
