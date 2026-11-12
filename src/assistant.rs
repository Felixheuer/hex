//! Labs › Assistant: a second, "Action" shortcut that asks a model by voice,
//! answers in the HUD and aloud, and may use the desktop; and Chats, the
//! window that lists every conversation and takes typed messages.
//!
//! The pieces, and who calls whom:
//!
//! ```ts
//! Action shortcut (recognition) ─┐                       ┌─ speech: reads the answer aloud
//!                                ├─ service::submit ─ agent loop ─ provider (claude | opencode)
//! Chats pane (rune_ui)  ─────────┘        │                 └─ computer: screenshot, click, type
//!                                         └─ store: conversations on disk
//! service::subscribe ─ Event ─ HUD bubble, Chats pane
//! ```
//!
//! Everything here is inert while `labs.assistant` is off.

// Until the Action shortcut and Chats call into it. Remove when wired.
#![allow(dead_code, unused_imports)]

pub mod agent;
pub mod computer;
pub mod credentials;
pub mod provider;
pub mod service;
pub mod speech;
pub mod store;
pub mod types;

pub use service::{cancel, conversation, conversations, delete, focus, set_focus, submit, subscribe};
pub use types::*;

use crate::app_settings::AppSettings;

/// Whether the assistant may run: its Lab is on and this OS has it.
pub fn enabled(settings: &AppSettings) -> bool {
    crate::labs::Feature::Assistant.on(settings)
}
