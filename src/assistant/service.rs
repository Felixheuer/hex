//! The assistant's one background worker: takes requests, runs them one at
//! a time through `agent`, saves them through `store`, and tells every
//! subscriber what happens. Callable from any thread.
//!
//! OWNER: brain (agent loop, providers, store). The signatures here are the
//! contract the Action shortcut and the Chats pane build against.

use super::types::{ConversationId, ConversationSummary, Conversation, Event, Request};

/// Called on the worker's thread for every event; keep it quick (post to
/// your own thread or wake your loop).
pub type Subscriber = Box<dyn Fn(&Event) + Send + Sync>;

/// Queues `request` and returns its conversation's id at once (a new id when
/// `request.conversation` is `None`). A request while another runs cancels
/// the running one first.
pub fn submit(request: Request) -> ConversationId {
    let _ = request;
    String::new()
}

/// Stops the running request, if any: its model call, its desktop actions,
/// and any speech. It ends with `Event::Cancelled`.
pub fn cancel() {}

/// Whether a request is running now.
pub fn busy() -> bool {
    false
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

/// The conversation Chats shows, so the Action shortcut continues it.
pub fn set_focus(id: Option<ConversationId>) {
    let _ = id;
}

/// The conversation the next Action press should continue: the one Chats
/// shows, else the last Action conversation answered within
/// `follow_up_seconds`, else `None` (start a new one).
pub fn focus(follow_up_seconds: u32) -> Option<ConversationId> {
    let _ = follow_up_seconds;
    None
}
