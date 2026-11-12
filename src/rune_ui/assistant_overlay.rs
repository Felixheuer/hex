//! Labs › Assistant's overlays: what the assistant draws over the desktop
//! (macOS).
//!
//! - The screen frame ([`frame`]): while Agent mode is on, a soft glow
//!   breathes around every display.
//! - Window outlines ([`outline`]): each running task's windows in its own
//!   colour ([`palette`]), with its title.
//! - The task stack ([`stack`]): Agent mode's tasks in a bottom corner, with
//!   their step, × to stop one, and a press to open it in Chats.
//! - The quick answer ([`bubble`]): a quick question's thinking, steps, and
//!   answer, just past the dictation HUD.
//!
//! [`board`] turns `assistant::Event`s into what these show; [`host`] makes
//! them Rune panels and feeds them from `assistant::subscribe`. The service
//! sends nothing while the Lab is off, so with it off nothing shows and no
//! panel or thread exists beyond the hidden overlays' bookkeeping.
//! [`fixture`] and [`desk`] are made-up activity and a pretend desktop for
//! the playground ([`stories`]) and the render tests.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod board;
mod bubble;
mod desk;
mod fixture;
mod frame;
#[cfg(target_os = "macos")]
mod host;
mod outline;
mod palette;
mod stack;
#[cfg(target_os = "macos")]
pub(crate) mod stories;
#[cfg(test)]
mod tests;

#[cfg(target_os = "macos")]
pub(crate) use host::install;
// For the Chats pane to call once it can open a conversation.
#[cfg(target_os = "macos")]
#[allow(unused_imports)]
pub(crate) use host::set_open_chats;
/// A task's colour, for Chats' running tasks.
pub(super) use palette::task as task_colour;
