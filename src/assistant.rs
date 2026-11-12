//! The assistant (on by default, `assistant.enabled`): a second, "Action"
//! shortcut. Hold it to ask by voice (the answer shows in the HUD and is
//! read aloud); double-tap it for Agent
//! mode, where a frame circles the screen and every hold starts another task
//! that runs beside the others, each operating its own windows with its own
//! cursor. Chats lists every conversation and takes typed messages.
//!
//! Hex does the voice, HUD, frame, and Chats. The thinking is done by an
//! agent already on this Mac (Claude Code, Codex, OpenCode) or the Claude
//! API; the desktop is operated through agent-desktop (Vercel Labs), which
//! acts through the accessibility tree without taking the pointer.
//!
//! ```ts
//! Action shortcut (recognition) ─┐                        ┌─ backend: claude -p | codex exec | opencode | API
//!                                ├─ service::submit ─ agent ┤      └─ agent-desktop (MCP / CLI), own cursor per task
//! Chats pane (rune_ui)  ─────────┘        │                 └─ speech: reads quick answers aloud
//!                                         └─ store: conversations on disk
//! service::subscribe ─ Event ─ HUD bubble and task stack, screen frame, window outlines, Chats
//! backends::detect ─ setup sheet (agents found, agent-desktop, permissions)
//! ```
//!
//! Everything here is inert while `assistant.enabled` is off.

// Parts only some OSes and the CLI call (`main.rs` allows dead code off
// macOS).
#![cfg_attr(target_os = "macos", allow(dead_code))]
#![allow(unused_imports)]

pub mod agent;
pub mod autonomy;
pub mod backends;
pub mod browser;
pub mod cli;
pub mod computer;
pub mod credentials;
pub mod guard;
pub mod memory;
pub mod notify;
pub mod palette;
pub mod provider;
pub mod schedules;
pub mod secrets;
pub mod service;
pub mod shortcut;
pub mod space;
pub mod speech;
pub mod store;
pub mod tools;
pub mod types;

pub use service::{
    agent_mode, background_busy, busy, cancel, cancel_all, conversation, conversations, delete,
    focus, hand_back, prewarm, resume_interrupted, set_agent_mode, set_focus, stop_all, submit,
    subscribe, take_over, tasks,
};
pub use types::*;

use crate::app_settings::AppSettings;

/// Whether the assistant may run: this OS has it (macOS, Linux, and
/// Windows) and its switch is on.
pub fn enabled(settings: &AppSettings) -> bool {
    available() && settings.assistant.enabled
}

/// Whether this OS has the assistant: every desktop Hex runs on.
pub const fn available() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
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
