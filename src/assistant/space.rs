//! Experimental (`assistant.agent_space`): a Space of its own for the windows
//! Background tasks open, so they stay out of the person's way, with the
//! task stack bringing one forward on request. macOS has no public API for
//! moving windows between Spaces; whether this can work reliably is decided
//! by `probe`, run only when the person asks (`hex dev assistant-space-probe`).
//!
//! OWNER: browser (with the Space spike).

/// Whether moving task windows to their own Space works on this Mac.
pub fn available() -> bool {
    false
}

/// Moves `window` to the agents' Space; false when it couldn't.
pub fn park(window: &crate::assistant::TaskWindow) -> bool {
    let _ = window;
    false
}

/// Brings `window` to the person's current Space and front.
pub fn show(window: &crate::assistant::TaskWindow) -> bool {
    let _ = window;
    false
}
