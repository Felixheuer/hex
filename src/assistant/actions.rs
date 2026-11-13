//! The fast action layer (roadmap section A): the quickest way to an app's
//! window or an app's action, before any click.
//!
//! - [`recovery`]: an app whose only window isn't on this screen (another
//!   desktop, minimized, the app hidden) gets one here without the person
//!   doing anything: shown, restored, a new window made, or (for apps with
//!   nothing unsaved, such as messengers) quit quietly and reopened in the
//!   background.
//! - [`app_action`]: any installed app's action by its fastest way
//!   (`hex_app_action`): a URL it takes (`open_url`) or AppleScript
//!   (`script`), checked by the guard like any action.
//! - [`capabilities`]: what any app offers, read from the app itself
//!   (`hex_app_capabilities`): its scripting dictionary, URL schemes,
//!   documents, and what its windows are made of. No list of apps.
//! - [`ocr`]: a window's text without Accessibility (`hex_read_window`), from
//!   its picture through the Vision framework, for apps such as Citrix.
//!
//! The guard (`assistant::guard`) runs all of them for a task; nothing here
//! decides whose window it is.
//!
//! OWNER: actions.

pub mod app_action;
pub mod ax;
pub mod capabilities;
pub mod ocr;
pub mod recovery;
