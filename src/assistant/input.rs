//! Input to an app's window in the background, as Codex works: the window
//! stays where it is (parked off the screen, or behind the person's), its
//! picture comes from ScreenCaptureKit, and clicks, scrolls, and keys go to
//! the app's process (its pid), never through the person's pointer or
//! keyboard. The person's cursor doesn't move, their app keeps the front and
//! the keyboard, and no window comes up over their work.
//!
//! What a window takes was measured on this Mac (macOS 26, TextEdit, a
//! throwaway document, while another app kept the front):
//!
//! | window is      | Accessibility | posted keys | posted clicks | picture |
//! |----------------|---------------|-------------|---------------|---------|
//! | behind another | yes           | yes         | yes (*)       | yes     |
//! | off the screen | yes           | yes         | yes (*)       | yes     |
//! | minimized      | yes           | no          | no            | no      |
//!
//! (*) only with the window stamped as the event's target
//! (`CGEventSetWindowLocation`, SkyLight's integer fields) and its app told
//! it has focus without being brought forward (one `SLPSPostEventRecordTo`
//! focus record plus a primer click outside the window, which eats AppKit's
//! first-click activation). Without them a background app drops the first
//! click of every gesture. Those calls are private (SkyLight); each is
//! looked up at run time ([`post::Capabilities`]) and missing ones turn
//! clicks off rather than fail, while keys and scrolls need none.
//!
//! So a task's window is never left minimized: [`decide`] keeps it parked
//! off the screen, or behind the person's windows when it doesn't render or
//! take input there, by what was measured for that window, never by app.
//!
//! - [`geometry`]: a window shot's pixels to the window's points and to the
//!   screen (scale-aware). Pure.
//! - [`decide`]: where a window waits, and when an Accessibility failure
//!   falls back to pid input. Pure.
//! - [`pointer`]: where the agent last clicked in a window, for the live
//!   picture in the overlay (a ring drawn into it).
//! - [`post`]: the pid-targeted events (macOS).
//! - [`shot`]: a window's picture at full resolution, as a PNG.
//!
//! OWNER: actions.

pub mod decide;
pub mod geometry;
pub mod pointer;
pub mod post;
pub mod shot;
