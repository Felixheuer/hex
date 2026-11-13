//! The person's iPhone while they're away: questions, approvals, failures
//! and finished jobs go there by iMessage, and their replies come back as
//! if typed here ("ich gehe jetzt zum Friseur" — and the job still reaches
//! them).
//!
//! ```ts
//! away      away::is_away: the switch, the screen locked, idle for
//!           assistant.phone_away_minutes, or „ich gehe jetzt …“ said
//! out       on_event (from the runner's events) → outgoing::message_for
//!           → send::send (AppleScript to Messages, the iMessage account)
//! in        poll thread: incoming::new_replies (~/Library/Messages/chat.db,
//!           only the configured handle, only rows after the away start;
//!           needs Full Disk Access) → route::route → submit / stop / status
//! ```
//!
//! Only `assistant.phone_handle` is written to or read from; message bodies
//! are never logged.
//!
//! OWNER: brain.

pub mod away;
pub mod incoming;
pub mod outgoing;
pub mod route;
pub mod send;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::app_settings::AssistantSettings;
use crate::assistant::Event;
use crate::desktop::automation::AutomationState;

/// How often replies are looked for while away.
const POLL: Duration = Duration::from_secs(4);

/// The configured handle, when the channel is on and the handle is usable.
fn configured(settings: &AssistantSettings) -> Option<String> {
    if !settings.phone_enabled {
        return None;
    }
    settings
        .phone_handle
        .as_deref()
        .and_then(incoming::clean_handle)
}

fn settings() -> Option<AssistantSettings> {
    crate::app_settings::assistant_runtime().map(|(_, settings)| settings)
}

/// A runner event, for the person's phone if they're away: a question, a
/// failure, or a Background job's result. `title` is the task's request.
pub fn on_event(event: &Event, title: &str) {
    let Some(settings) = settings() else {
        return;
    };
    let Some(handle) = configured(&settings) else {
        return;
    };
    if !away::is_away_now(settings.phone_away_minutes) {
        return;
    }
    let Some(message) = outgoing::message_for(event, title) else {
        return;
    };
    send_async(handle, message);
}

/// A request the person made here: „ich gehe jetzt …“ starts away mode.
pub fn note_request(text: &str) {
    if away::says_leaving(text) {
        away::left_now();
        start();
    }
}

/// What the test button sends.
pub fn test_message() -> String {
    outgoing::test_message()
}

/// Sends [`test_message`] to the configured handle (the Settings button;
/// tests never send). Whether the handle was usable.
pub fn send_test(settings: &AssistantSettings) -> bool {
    let Some(handle) = settings
        .phone_handle
        .as_deref()
        .and_then(incoming::clean_handle)
    else {
        return false;
    };
    send_async(handle, test_message());
    true
}

/// Whether Hex may control Messages, once asked from Settings.
static MESSAGES: Mutex<Option<AutomationState>> = Mutex::new(None);

pub fn messages_permission() -> Option<AutomationState> {
    *MESSAGES.lock().unwrap_or_else(|error| error.into_inner())
}

/// Asks macOS (once it shows a prompt) whether Hex may control Messages,
/// off the main thread.
pub fn ask_messages() {
    let _ = std::thread::Builder::new()
        .name("phone-permission".into())
        .spawn(|| {
            let state = crate::desktop::automation::messages(true);
            *MESSAGES.lock().unwrap_or_else(|error| error.into_inner()) = Some(state);
        });
}

fn send_async(handle: String, message: String) {
    let _ = std::thread::Builder::new()
        .name("phone-send".into())
        .spawn(move || {
            if let Err(error) = send::send(&handle, &message) {
                tracing::warn!(%error, "couldn't send to the person's phone");
            }
        });
}

static STARTED: AtomicBool = AtomicBool::new(false);
/// The newest chat.db row already handled, and when away mode began.
static CURSOR: Mutex<Option<(i64, std::time::SystemTime)>> = Mutex::new(None);

/// Starts the reply poller once (it idles while the channel is off or the
/// person is here). The app calls it at launch.
pub fn start() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("phone-replies".into())
        .spawn(|| {
            loop {
                std::thread::sleep(POLL);
                poll_once();
            }
        });
}

fn poll_once() {
    let Some(settings) = settings() else {
        return;
    };
    let Some(handle) = configured(&settings) else {
        *CURSOR.lock().unwrap_or_else(|error| error.into_inner()) = None;
        return;
    };
    let away = away::is_away_now(settings.phone_away_minutes);
    let mut cursor = CURSOR.lock().unwrap_or_else(|error| error.into_inner());
    if !away {
        *cursor = None;
        return;
    }
    // Away begins: replies count from now on, never older ones.
    let (after, since) = match *cursor {
        Some(state) => state,
        None => {
            let Ok(newest) = incoming::newest_rowid() else {
                return;
            };
            let state = (newest, std::time::SystemTime::now());
            *cursor = Some(state);
            return;
        }
    };
    let replies = match incoming::new_replies(&handle, after) {
        Ok(replies) => replies,
        Err(error) => {
            tracing::debug!(%error, "couldn't read replies from Messages");
            return;
        }
    };
    let Some(last) = replies.last().map(|reply| reply.rowid) else {
        return;
    };
    *cursor = Some((last, since));
    drop(cursor);
    for reply in replies.into_iter().filter(|reply| reply.sent_after(since)) {
        tracing::info!(
            chars = reply.text.chars().count(),
            "a reply from the person's phone"
        );
        route::carry_out(&handle, &reply.text);
    }
}
