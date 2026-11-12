//! The assistant's task runner: takes requests, runs up to `max_parallel`
//! at once (each through `agent` on its backend), queues the rest, saves
//! conversations through `store`, owns Agent mode's on/off state and its
//! agent-desktop multi-agent session, and tells every subscriber what
//! happens. Callable from any thread.
//!
//! OWNER: brain. The signatures here are the contract the Action shortcut,
//! the HUD, and the Chats pane build against.

use super::types::{
    Conversation, ConversationId, ConversationSummary, Event, Request, TaskStatus,
};

/// Called on a worker thread for every event; keep it quick (post to your
/// own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

/// Queues `request` and returns its conversation's id at once (a new id when
/// `request.conversation` is `None`). A quick question (`Origin::ActionShortcut`)
/// cancels the previous quick question still running; Agent mode and Chat
/// tasks run beside each other.
pub fn submit(request: Request) -> ConversationId {
    let _ = request;
    String::new()
}

/// Stops one task: its agent process, its desktop actions, and its speech.
/// It ends with `Event::Cancelled`.
pub fn cancel(conversation: &str) {
    let _ = conversation;
}

/// Stops every task.
pub fn cancel_all() {}

/// The tasks running or queued now, oldest first.
pub fn tasks() -> Vec<TaskStatus> {
    Vec::new()
}

/// Whether any task is running or queued.
pub fn busy() -> bool {
    false
}

/// Whether Agent mode is on.
pub fn agent_mode() -> bool {
    false
}

/// Turns Agent mode on or off (`Event::ModeChanged`). Turning it off lets
/// running tasks finish; their outlines and cursors stay until they do.
pub fn set_agent_mode(on: bool) {
    let _ = on;
}

/// Calls `subscriber` for every later event, until the process ends.
pub fn subscribe(subscriber: Subscriber) {
    let _ = subscriber;
}

/// Every stored conversation, newest first.
pub fn conversations() -> Vec<ConversationSummary> {
    Vec::new()
}

pub fn conversation(id: &str) -> Option<Conversation> {
    let _ = id;
    None
}

pub fn delete(id: &str) {
    let _ = id;
}

/// The conversation Chats shows, so a quick question continues it.
pub fn set_focus(id: Option<ConversationId>) {
    let _ = id;
}

/// The conversation the next quick question should continue: the one Chats
/// shows, else the last quick conversation answered within
/// `follow_up_seconds`, else `None` (start a new one). Agent mode never
/// continues one.
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    let _ = follow_up_seconds;
    None
}
