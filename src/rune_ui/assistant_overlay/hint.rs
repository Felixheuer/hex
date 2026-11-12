//! What the HUD says past itself while the Action shortcut is held:
//!
//! - tapped then held (a Background task): a pill, "✦ New background task",
//!   so the hold reads as starting a task that runs beside the others;
//! - held (a Here question): no pill, only a tiny "Here · Mail" naming the
//!   app in front, the one the question is about and may act in.
//!
//! Every Action capture also tints the HUD's dots in the assistant's
//! periwinkle (`dot_scope::palette`).

use rune::{Cx, El, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::SPARKLES;
use super::palette;
use crate::desktop::i18n::{tr, tr_fill};

/// The overlay: room for the pill in any language.
pub(crate) const SIZE: (f32, f32) = (300.0, 40.0);
const PILL_H: f32 = 28.0;
const LABEL_H: f32 = 20.0;

/// What the hint says.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum Hint {
    /// "New background task".
    #[default]
    Background,
    /// "Here · <app>".
    Here(String),
}

pub(crate) fn view(hint: &Hint, cx: &mut Cx<Hint>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    row()
        .key("hint-root")
        .w_full()
        .h_full()
        .center()
        .child(drawn(p, hint))
}

pub(crate) fn drawn(p: Paint, hint: &Hint) -> El {
    match hint {
        Hint::Background => pill(p),
        Hint::Here(app) => here(p, app),
    }
}

/// The pill: a sparkle in the assistant's colour and "New background task".
pub(crate) fn pill(p: Paint) -> El {
    let c = &p.k.c;
    let (lead, _) = palette::frame(p.k.dark);
    row()
        .key("hint")
        .h(PILL_H)
        .pl(10.0)
        .pr(12.0)
        .gap(6.0)
        .center_y()
        .radius(PILL_H / 2.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("status")
        .child(
            icon(&SPARKLES, 14.0)
                .key("sparkle")
                .color(lead)
                .a11y_hidden(),
        )
        .child(
            text(tr("New background task"))
                .key("hint-text")
                .font(p.k.ty.small_medium)
                .color(p.text()),
        )
}

/// The tiny label naming the app a Here question is about.
fn here(p: Paint, app: &str) -> El {
    let c = &p.k.c;
    row()
        .key("here")
        .h(LABEL_H)
        .px(8.0)
        .center_y()
        .radius(LABEL_H / 2.0)
        .fill(c.bg_base.alpha(0.82))
        .role("status")
        .child(
            text(tr_fill("Here · {}", app))
                .key("here-text")
                .truncate()
                .font(p.k.ty.small)
                .color(p.muted()),
        )
}
