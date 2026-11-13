//! What the stream shares about its entries: their icons by lane, and the
//! day they fall under.

use super::super::icons::{KEYBOARD, MIC, SPARKLES};
use crate::assistant::Origin;
use crate::desktop::i18n::tr;

const DAY: u64 = 24 * 60 * 60 * 1_000;

/// A step's icon in the transcript: something done on the desktop.
pub(super) static AGENT_ICON: &rune::Icon = &rune_theme_opencode::icons::WINDOW_CURSOR;
pub(super) static INTERRUPTED_ICON: &rune::Icon = &rune_theme_opencode::icons::WARNING;

/// The icon before an entry: its lane, and how it was asked.
pub(super) fn origin_icon(origin: Origin) -> &'static rune::Icon {
    match origin {
        Origin::ActionShortcut => &MIC,
        Origin::Background => &SPARKLES,
        Origin::Chat => &KEYBOARD,
    }
}

/// The day heading an entry falls under, by when it was asked.
pub(super) fn group(at_ms: u64, now_ms: u64) -> &'static str {
    match crate::meeting::day_label(at_ms, now_ms).as_str() {
        "Today" => tr("Today"),
        "Yesterday" => tr("Yesterday"),
        _ if now_ms.saturating_sub(at_ms) < 7 * DAY => tr("Previous 7 days"),
        _ => tr("Older"),
    }
}
