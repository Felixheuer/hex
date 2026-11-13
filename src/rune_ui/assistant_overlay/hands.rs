//! "Hands off": a calm pill just past the HUD while a task moves the real
//! pointer or presses real keys in the window in front, so the person
//! knows to let go of the mouse, and that Esc stops it. Its window gets a
//! soft outline meanwhile (`outline`, `Look::HandsOff`).

use rune::{Cx, El, Icon, row, text};

use super::super::controls::Paint;
use super::palette;
use crate::desktop::i18n::tr;

/// The overlay: room for the pill in any language.
pub(crate) const SIZE: (f32, f32) = (460.0, 44.0);
const PILL_H: f32 = 30.0;

/// A small arrow pointer.
static POINTER: Icon = Icon {
    name: "hex-hands-off-pointer",
    view: 24.0,
    body: "<path d=\"M5 3l13 7.5-5.6 1.4L9.8 18z\" fill=\"currentColor\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linejoin=\"round\"/>",
};

/// The pill's overlay state: whether it shows (it keeps drawing while it
/// fades out).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hands {
    /// The task with its hands on the pointer.
    pub(crate) task: Option<String>,
}

pub(crate) fn view(_: &Hands, cx: &mut Cx<Hands>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    row()
        .key("hands-root")
        .w_full()
        .h_full()
        .center()
        .child(pill(p))
}

/// The pill: a pointer in the assistant's colour, what's happening, and
/// how to stop it.
pub(crate) fn pill(p: Paint) -> El {
    let c = &p.k.c;
    let (lead, _) = palette::frame(p.k.dark);
    row()
        .key("hands-off")
        .h(PILL_H)
        .pl(11.0)
        .pr(13.0)
        .gap(7.0)
        .center_y()
        .radius(PILL_H / 2.0)
        .fill(c.bg_base)
        .border(1.0, lead.alpha(0.55))
        .outline(3.0, 0.0, lead.alpha(0.14))
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(icon_el(lead))
        .child(
            text(tr("Hands off: Agent-Desk is using your mouse"))
                .key("hands-text")
                .font(p.k.ty.small_medium)
                .color(p.text()),
        )
        .child(
            text(tr("Esc to stop"))
                .key("hands-esc")
                .font(p.k.ty.small)
                .color(p.faint()),
        )
}

fn icon_el(colour: rune::Color) -> El {
    rune::icon(&POINTER, 13.0)
        .key("hands-pointer")
        .color(colour)
        .a11y_hidden()
}
