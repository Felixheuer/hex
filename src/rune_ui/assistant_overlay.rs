//! The assistant's overlays: what it draws over the desktop.
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

mod board;
mod bubble;
mod cursor;
mod desk;
mod fixture;
mod frame;
mod hands;
mod hint;
mod host;
mod listening;
mod markdown;
#[cfg(test)]
mod multi;
mod orb;
mod outline;
mod palette;
mod place;
mod pops;
mod quick;
mod screens;
mod stack;
pub(crate) mod stories;
#[cfg(test)]
mod tests;
mod when;

pub(crate) use host::{ask_hint, listening};
use host::{install, set_open_chats, set_open_setup, setup_needed};

/// Task `colour`'s agent cursor (a soft arrow in its colour, drawn once
/// into `<support>/assistant/cursors/`), for agent-desktop's
/// `cursor-overlay enable --image`; `None` when it can't be drawn.
pub(crate) use cursor::cursor_image;

/// A task's colour, for Chats' running tasks.
pub(super) use palette::task as task_colour;

/// Adds the overlays to the app as it starts, and wires what opens from
/// them, from the Action shortcut, and from notifications: `open_chat`
/// opens Chats on a conversation (or on the most relevant one with `None`),
/// `open_setup` the assistant's setup where it was left. Both are called
/// from any thread. Every OS's app calls this once.
pub(crate) fn install_wired<S: 'static>(
    app: &mut rune::App<S>,
    open_chat: fn(Option<String>),
    open_setup: fn(),
) {
    // The frame, window outlines, task stack, and quick answer; hidden,
    // without windows, until the assistant reports work.
    install(app);
    // The task stack and "Open in Chats" open the conversation in Chats.
    set_open_chats(move |id| open_chat(Some(id.to_owned())));
    // A hold of the Action shortcut before setup says "Finish setting up
    // the assistant first" and opens the setup; the bubble's "Continue
    // setup" opens it too.
    set_open_setup(open_setup);
    crate::assistant::trigger::set_open_setup(move || {
        setup_needed();
        open_setup();
    });
    // A click on a notification opens its conversation (macOS opens a
    // `hex://chats` link instead).
    #[cfg(not(target_os = "macos"))]
    crate::assistant::notify::set_on_click(move |id| open_chat(Some(id.to_owned())));
}
