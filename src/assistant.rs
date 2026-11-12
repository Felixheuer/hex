//! Labs › Assistant: a second, "Action" shortcut. Hold it to ask by voice
//! (the answer shows in the HUD and is read aloud); double-tap it for Agent
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
//! Everything here is inert while `labs.assistant` is off.

// Until the Action shortcut and Chats call into it. Remove when wired.
#![allow(dead_code, unused_imports)]

pub mod agent;
pub mod backends;
pub mod browser;
pub mod cli;
pub mod computer;
pub mod credentials;
pub mod guard;
pub mod provider;
pub mod service;
pub mod space;
pub mod speech;
pub mod store;
pub mod types;

pub use service::{
    agent_mode, busy, cancel, cancel_all, conversation, conversations, delete, focus,
    set_agent_mode, set_focus, submit, subscribe, tasks,
};
pub use types::*;

use crate::app_settings::AppSettings;

/// Whether the assistant may run: its Lab is on and this OS has it.
pub fn enabled(settings: &AppSettings) -> bool {
    crate::labs::Feature::Assistant.on(settings)
}

/// Whether the assistant is on and its onboarding finished: until then,
/// Chats offers to finish setting it up instead of a chat.
/// (Stub for the lead's contract; the lead owns it at integration.)
pub fn ready(settings: &AppSettings) -> bool {
    enabled(settings) && settings.assistant.setup_done
}
