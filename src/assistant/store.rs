//! Conversations on disk: one JSON file per conversation, and its screenshots
//! beside it, under `<support dir>/assistant/conversations/`.
//!
//! OWNER: brain.

use std::path::PathBuf;

/// Where a conversation's attachment `file` lives.
pub fn attachment_path(conversation: &str, file: &str) -> Option<PathBuf> {
    let _ = (conversation, file);
    None
}
