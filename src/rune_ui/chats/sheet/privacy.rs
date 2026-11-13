//! What leaves the Mac and what never does, in three plain lines under the
//! welcome page's card. How freely the assistant acts, the blocked apps,
//! and what it knows about you are set in Assistant settings.

use rune::{El, col, icon, row, text};

use super::super::super::controls::Paint;
use crate::desktop::i18n::tr;

static SEND_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_UP_RIGHT;
static LOCAL_ICON: &rune::Icon = &rune_theme_opencode::icons::LOCK;
static SHIELD_ICON: &rune::Icon = &rune_theme_opencode::icons::SHIELD;

/// One plain-words line: an icon and a sentence.
fn fact(p: Paint, key: &str, glyph: &'static rune::Icon, words: &str) -> El {
    row()
        .key(key.to_owned())
        .gap(10.0)
        .items_start()
        .child(
            icon(glyph, 14.0)
                .key("icon")
                .color(p.k.c.icon_muted)
                .mt(1.0)
                .a11y_hidden(),
        )
        .child(
            text(words.to_owned())
                .key("words")
                .font(p.k.ty.small)
                .color(p.muted())
                .wrap()
                .grow(1.0)
                .shrink(),
        )
}

/// Where your words go, and what never leaves.
pub(super) fn facts(p: Paint) -> El {
    col()
        .key("facts")
        .role("group")
        .label(tr("Your privacy"))
        .gap(8.0)
        .child(fact(
            p,
            "goes",
            SEND_ICON,
            tr("Questions, screenshots, and page content go to the agent you choose (Claude, OpenAI, DeepSeek, …)."),
        ))
        .child(fact(
            p,
            "stays",
            LOCAL_ICON,
            if cfg!(target_os = "macos") {
                tr("Dictation stays on your Mac.")
            } else {
                tr("Dictation stays on your computer.")
            },
        ))
        .child(fact(
            p,
            "never",
            SHIELD_ICON,
            tr("Blocked apps and password fields are never captured."),
        ))
}
