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
pub mod service;
pub mod space;
pub mod speech;
pub mod store;
pub mod tools;
pub mod trigger;
pub mod types;

pub use service::{
    busy, cancel, conversation, conversations, delete, focus, set_focus, submit, subscribe, tasks,
};
pub use types::*;

use crate::app_settings::AppSettings;

/// Whether the assistant may run: this OS has it and its switch is on.
pub fn enabled(settings: &AppSettings) -> bool {
    cfg!(target_os = "macos") && settings.assistant.enabled
}

/// Whether the assistant may answer: enabled, and its setup finished. Until
/// then a hold of the Action shortcut opens the setup instead of asking.
pub fn ready(settings: &AppSettings) -> bool {
    enabled(settings) && settings.assistant.setup_done
}
