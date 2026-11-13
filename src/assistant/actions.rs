//! The fast action layer (roadmap section A): the quickest way to an app's
//! window or an app's action, before any click.
//!
//! - [`recovery`]: an app whose only window isn't on this screen (another
//!   desktop, minimized, the app hidden) gets one here without the person
//!   doing anything: shown, restored, a new window made, or (for apps with
//!   nothing unsaved, such as messengers) quit quietly and reopened in the
//!   background.
//! - [`recipes`]: deep links and app recipes (`hex_app_action`): a WhatsApp
//!   draft, a Slack channel, a note, a reminder, a Finder reveal, a Settings
//!   pane, … as data (`recipes.toml`), checked by the guard like any action.
//! - [`ocr`]: a window's text without Accessibility (`hex_read_window`), from
//!   its picture through the Vision framework, for apps such as Citrix.
//!
//! The guard (`assistant::guard`) runs all of them for a task; nothing here
//! decides whose window it is.
//!
//! OWNER: actions.

pub mod ax;
pub mod ocr;
pub mod recipes;
pub mod recovery;
