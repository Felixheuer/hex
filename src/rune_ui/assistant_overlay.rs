//! The assistant's overlays: what it draws over the desktop (macOS).
//!
//! - The screen frame ([`frame`]): while Background work runs, waits, or is
//!   queued, a soft glow breathes around every display.
//! - Window outlines ([`outline`]): each live task's windows in its own
//!   colour ([`palette`]), with its title; dashed with "You have it" while
//!   the person took it over, dimmed while it waits out a usage limit, soft
//!   while a task moves the real pointer there.
//! - The orb ([`orb`]): Background tasks' quiet presence beside the HUD (or
//!   where it was dragged, [`place`]), one dot per task; bubbles pop from
//!   it ([`pops`]) and its list unfolds from it ([`stack`]) with Take over,
//!   Hand back, Stop all, and each plan's checklist. A task's question
//!   stays beside it ("Hold ⌘ to answer"); a plan rings its dot with
//!   progress; an escalated Here question flies in as a new dot.
//! - The agents' cursors ([`cursor`]): a soft arrow per task colour, for
//!   agent-desktop's `--image`. The colours themselves are
//!   `assistant::palette`, shared with the brain.
//! - The Here question's bubble ([`bubble`]) just past the dictation HUD,
//!   growing out of it and pulsing while the answer is read aloud; the
//!   "Finish setting up" bubble in its place before setup.
//! - Past the HUD while the Action shortcut is held ([`hint`]): "New
//!   background task" for a tap+hold, "Here · Mail" for a hold; and the
//!   "Hands off" pill ([`hands`]) while a task moves the real pointer.
//!
//! [`board`] turns `assistant::Event`s into what these show; [`host`] makes
//! them Rune panels and feeds them from `assistant::subscribe`. The service
//! sends nothing while the assistant is off, so with it off nothing shows
//! and no panel or thread exists beyond the hidden overlays' bookkeeping.
//! [`fixture`] and [`desk`] are made-up activity and a pretend desktop for
//! the playground ([`stories`]) and the render tests.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod board;
mod bubble;
#[cfg(target_os = "macos")]
mod cursor;
mod desk;
mod fixture;
mod frame;
mod hands;
mod hint;
#[cfg(target_os = "macos")]
mod host;
mod markdown;
mod orb;
mod outline;
mod palette;
mod place;
mod pops;
mod quick;
mod stack;
#[cfg(target_os = "macos")]
pub(crate) mod stories;
#[cfg(test)]
mod tests;
mod when;

#[cfg(target_os = "macos")]
pub(crate) use host::{ask_hint, install, set_open_chats, set_open_setup, setup_needed};

/// A task's colour, for Chats' running tasks.
pub(super) use palette::task as task_colour;

/// Unfolds the task list from the orb, or folds it back: the Action
/// shortcut's double-tap. Any thread; nothing while no Background task is
/// listed, and nothing off macOS.
#[allow(dead_code)] // The Action shortcut calls it through `ask::set_toggle_tasks`.
pub fn toggle_tasks() {
    #[cfg(target_os = "macos")]
    host::toggle_tasks();
}

/// Task `colour`'s agent cursor (a soft arrow in its colour, drawn once
/// into `<support>/assistant/cursors/`), for agent-desktop's
/// `cursor-overlay enable --image`. `None` when it can't be drawn, and off
/// macOS.
#[allow(dead_code)] // The brain passes it to agent-desktop.
pub fn cursor_image(colour: u8) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    return cursor::cursor_image(colour);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = colour;
        None
    }
}
