//! "New task": a small pill just past the dictation HUD while the Action
//! shortcut is held in Agent mode, so the hold reads as starting another
//! task rather than dictating. (Every Action capture also tints the HUD's
//! dots in the assistant's colours: `dot_scope::palette`.)

use rune::{Cx, El, icon, row, text};

use super::super::controls::Paint;
use super::super::icons::SPARKLES;
use super::palette;
use crate::desktop::i18n::tr;

/// The overlay: room for the pill in any language.
pub(crate) const SIZE: (f32, f32) = (260.0, 40.0);
const PILL_H: f32 = 28.0;

/// The hint's overlay state: nothing changes while it shows.
#[derive(Default)]
pub(crate) struct Hint;

pub(crate) fn view(_: &Hint, cx: &mut Cx<Hint>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    row()
        .key("hint-root")
        .w_full()
        .h_full()
        .center()
        .child(pill(p))
}

/// The pill: a sparkle in the assistant's colour and "New task".
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
            text(tr("New task"))
                .key("hint-text")
                .font(p.k.ty.small_medium)
                .color(p.text()),
        )
}
