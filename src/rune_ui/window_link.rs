//! Agent-Desk's window from any thread, on every OS, and the assistant's
//! ways to it. The window opens only when the person asks: the stage's
//! "Open in Agent-Desk", "Continue setup", Fix for what only the setup
//! repairs, the repair card's Fix all…, a notification's own "Open
//! Agent-Desk" action, and (elsewhere) the menu bar, the Dock, `agent-desk
//! window`, and links. Everything the assistant shows by itself goes to the
//! overlays instead: a notification's click projects its result on the
//! stage, a hold before setup says "Finish setting up" in the bubble, "Turn
//! on" switches the assistant on in place. `code_shape_tests` keeps every
//! caller of these openers on its allow-list.
//!
//! macOS goes through `macos_rune`, which also brings the app forward; Linux
//! and Windows through the window's app handle, remembered as the app
//! starts ([`remember`]).

#[cfg(not(target_os = "macos"))]
use std::sync::OnceLock;

#[cfg(not(target_os = "macos"))]
use super::{Destination, Ui};

#[cfg(not(target_os = "macos"))]
static WINDOW: OnceLock<rune::AppHandle<Ui>> = OnceLock::new();

/// How often an opener ran (tests: nothing opens by itself).
#[cfg(test)]
pub(crate) static OPENED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn opening() {
    #[cfg(test)]
    OPENED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Remembers the window's app handle (Linux and Windows), once.
#[cfg(not(target_os = "macos"))]
pub(crate) fn remember(window: rune::AppHandle<Ui>) {
    let _ = WINDOW.set(window);
}

/// Opens the window on Chats, on `conversation` when given.
pub(crate) fn open_chat(conversation: Option<String>) {
    opening();
    #[cfg(target_os = "macos")]
    crate::macos_rune::open_chat(conversation);
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
        window.update(move |ui, tx| super::show(ui, Destination::Chats(conversation), tx));
    }
}

/// Opens the assistant's setup where it was left.
pub(crate) fn open_assistant_setup() {
    opening();
    #[cfg(target_os = "macos")]
    crate::macos_rune::open_assistant_setup();
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
        window.update(super::open_assistant_setup);
    }
}

/// The repair card's Fix all…: the window on the guide.
pub(crate) fn open_repair() {
    opening();
    #[cfg(target_os = "macos")]
    crate::macos_rune::open_repair();
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.open();
        window.update(|ui, tx| super::repair_asked(ui, true, tx));
    }
}

/// Changes the window's state without opening it.
fn quietly(change: fn(&mut super::Ui, &mut rune::Tx)) {
    #[cfg(target_os = "macos")]
    crate::macos_rune::update_window(change);
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = WINDOW.get() {
        window.update(change);
    }
}

/// A hold of the Action shortcut before setup is done: "Finish setting up"
/// with Continue setup where the Here bubble shows. The window stays shut.
pub(crate) fn hold_before_setup() {
    super::assistant_overlay::setup_needed();
}

/// A notification's click: the job's result on the stage; its "Open
/// Agent-Desk" action (`window`) the window on that conversation.
pub(crate) fn notification_clicked(conversation: &str, window: bool) {
    if window {
        open_chat(Some(conversation.to_owned()));
    } else {
        super::assistant_overlay::show_result(conversation);
    }
}

/// Wires every way the assistant reaches the window (and the ways that no
/// longer do), on every OS.
pub(crate) fn wire_assistant() {
    use super::assistant_overlay::{self as overlay, Repair};
    // Behind ⋯ on the stage: the person asked.
    overlay::set_open_chats(|id| open_chat(Some(id.to_owned())));
    // Continue setup, Fix for what only the setup repairs: presses.
    overlay::set_open_setup(open_assistant_setup);
    overlay::set_turn_on(|| quietly(super::turn_on_assistant));
    overlay::set_repair(|ask| match ask {
        Repair::FixAll => open_repair(),
        Repair::Later => quietly(|ui, tx| super::repair_asked(ui, false, tx)),
    });
    crate::assistant::shortcut::set_open_setup(hold_before_setup);
    crate::assistant::notify::set_open(notification_clicked);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    /// The counter is shared: one test at a time looks at it.
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_hold_before_setup_does_not_open_the_window() {
        let _one = ONE.lock().unwrap_or_else(|error| error.into_inner());
        let before = OPENED.load(Ordering::SeqCst);
        hold_before_setup();
        crate::assistant::shortcut::set_open_setup(hold_before_setup);
        crate::assistant::shortcut::open_setup();
        assert_eq!(OPENED.load(Ordering::SeqCst), before, "no window");
    }

    #[test]
    fn a_notification_click_shows_the_result_on_the_stage() {
        let _one = ONE.lock().unwrap_or_else(|error| error.into_inner());
        let before = OPENED.load(Ordering::SeqCst);
        notification_clicked("suppliers", false);
        assert_eq!(OPENED.load(Ordering::SeqCst), before, "no window");
        assert!(
            super::super::assistant_overlay::projected().contains(&"suppliers".to_owned()),
            "its result goes to the stage"
        );
        // Its own "Open Agent-Desk" action is the deliberate way.
        notification_clicked("suppliers", true);
        assert!(OPENED.load(Ordering::SeqCst) > before);
    }
}
