//! The assistant's overlays: what it draws over the desktop (macOS).
//!
//! - Three frames, one meaning each (`docs/ux/redesign.md` §3.5): the desk
//!   frame ([`frame`]), a still glow around every display with its tab,
//!   only while the agent has the person's input or a window in front;
//!   window outlines ([`outline`]), only around windows on the screen a
//!   live task works in, in its colour ([`palette`]) with a chip naming it
//!   in every look (dashed while the person took it over, dimmed while it
//!   waits out a usage limit); and the ring around a parked window's mini
//!   in the rail ([`pictures::ring`]).
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
//! - The island ([`island`]): the conversation's black capsule, one line of
//!   state (Listening, Understood, the step now, Needs you, ✓, ✗), with the
//!   live words under it while the Action shortcut is held and the
//!   reasoning lines (Understood, Plan, Now, Next) while it thinks.
//! - The island at the notch ([`island`]) with the Here question's card
//!   ([`bubble`]) under it, pulsing while the answer is read aloud; the
//!   "Finish setting up" card in its place before setup.
//! - Past the HUD while the Action shortcut is held ([`hint`]): "New
//!   background task" for a tap+hold, "Here · Mail" for a hold. "Hands
//!   off" is the desk frame's tab ([`frame`]) under the island.
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
mod desk;
mod enlarged;
mod failure;
mod fixture;
mod frame;
#[cfg(test)]
mod frames_tests;
mod hint;
mod host;
mod island;
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
mod project;
mod quick;
mod results;
mod screens;
mod stack;
mod stage;
#[cfg(target_os = "macos")]
pub(crate) mod stories;
#[cfg(test)]
mod tests;
mod wall;
mod when;

#[cfg(test)]
pub(crate) use host::projected;
pub(crate) use host::{
    Repair, ask_hint, install, listening, needs_repair, set_open_chats, set_open_setup, set_repair,
    set_turn_on, setup_needed, show_result, stage_takes,
};

/// A task's colour, for Chats' running tasks.
pub(super) use palette::task as task_colour;

/// Unfolds the task list from the orb, or folds it back: the Action
/// shortcut's double-tap. Any thread; nothing while no Background task is
/// listed.
#[allow(dead_code)] // The Action shortcut calls it through `shortcut::set_toggle_tasks`.
pub fn toggle_tasks() {
    host::toggle_tasks();
}
