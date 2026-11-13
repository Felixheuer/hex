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
//!   Hand back, Stop all, and each plan's checklist. Running jobs show as a
//!   few compact rows with their step and live pictures ([`jobs`]); a
//!   task's question stays beside it, answered by voice or typed there
//!   ([`ask`]); a finished job leaves a result bubble with what to open or
//!   Try again, recalled from the orb for a while ([`results`]); a plan
//!   rings its dot with progress; an escalated Here question flies in as a
//!   new dot. Nothing needs the window.
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
//! [`board`] adds the orb's bubbles and results over the shared task view
//! (`assistant::view`, which the Assistant pane reads too); [`host`] makes
//! them Rune panels and feeds them from `assistant::subscribe`. The service
//! sends nothing while the assistant is off, so with it off nothing shows
//! and no panel or thread exists beyond the hidden overlays' bookkeeping.
//! [`fixture`] and [`desk`] are made-up activity and a pretend desktop for
//! the playground ([`stories`]) and the render tests.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

pub(crate) mod app_live;
mod ask;
mod board;
mod bubble;
#[cfg(target_os = "macos")]
mod cursor;
mod desk;
mod enlarged;
mod failure;
mod fixture;
mod frame;
mod hands;
mod hint;
mod host;
mod jobs;
mod listening;
pub(crate) mod live;
pub(crate) mod live_card;
mod look;
mod markdown;
mod minis;
#[cfg(test)]
mod multi;
mod orb;
mod outline;
#[cfg(test)]
mod overview_tests;
mod palette;
pub(crate) mod pictures;
mod place;
mod pops;
mod quick;
mod results;
mod screens;
mod stack;
#[cfg(target_os = "macos")]
pub(crate) mod stories;
#[cfg(test)]
mod tests;
mod wall;
mod when;

pub(crate) use host::{ask_hint, install, listening, set_open_chats, set_open_setup, setup_needed};

/// A task's colour, for Chats' running tasks.
pub(super) use palette::task as task_colour;

/// Unfolds the task list from the orb, or folds it back: the Action
/// shortcut's double-tap. Any thread; nothing while no Background task is
/// listed.
#[allow(dead_code)] // The Action shortcut calls it through `shortcut::set_toggle_tasks`.
pub fn toggle_tasks() {
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
