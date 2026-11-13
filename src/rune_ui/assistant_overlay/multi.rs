//! This Mac's two displays, pretend, for the render tests: the built-in
//! display (primary) and the M27Q above it and to its left, with the frame
//! on both joined along their seam, an outline on each (and one across
//! the seam), and the orb beside the HUD on the display the person works
//! on (the M27Q). Drawn in global points shifted so the desk's top-left is
//! at (0, 0).

use rune::{Cx, El, el, row, text};

use super::super::controls::Paint;
use super::board::{Board, Look};
use super::outline::{self, Outline};
use super::screens::{self, Screen};
use super::{fixture, frame, orb, place};

/// The desk this Mac's displays span, in global points.
pub(crate) fn screens() -> Vec<Screen> {
    vec![
        Screen {
            id: 1,
            key: "built-in".into(),
            frame: [0.0, 0.0, 1800.0, 1169.0],
            visible: [0.0, 37.0, 1800.0, 1132.0],
            scale: 2.0,
        },
        Screen {
            id: 2,
            key: "m27q".into(),
            frame: [-604.0, -1440.0, 2560.0, 1440.0],
            visible: [-604.0, -1415.0, 2560.0, 1345.0],
            scale: 2.0,
        },
    ]
}

/// The desk's top-left and size.
const ORIGIN: (f64, f64) = (-604.0, -1440.0);
pub(crate) const SIZE: (f32, f32) = (2560.0, 2609.0);
const MENU_BAR: f32 = 25.0;
const HUD: (f32, f32) = (132.0, 34.0);

/// The scene's state: the tasks the orb shows.
pub(crate) struct TwoDisplays {
    pub(crate) board: Board,
}

/// A window's outline: its colour, title, look, and plan progress.
type Outlining = (u8, &'static str, Look, Option<(usize, usize)>);

/// The windows on the desk: their app, global rect, and outline.
fn windows() -> [(&'static str, [f64; 4], Option<Outlining>); 4] {
    [
        (
            "Safari",
            [-420.0, -1180.0, 1100.0, 720.0],
            Some((
                0,
                "Find a table for two at Nopa tonight",
                Look::Working,
                Some((2, 5)),
            )),
        ),
        (
            "Mail",
            [950.0, 260.0, 720.0, 520.0],
            Some((
                1,
                "Draft replies to the three unread emails…",
                Look::Paused,
                None,
            )),
        ),
        (
            "Arc",
            [160.0, -330.0, 760.0, 560.0],
            Some((
                5,
                "Summarise today's top stories",
                Look::Waiting {
                    until_ms: super::when::now_ms() + 23 * 60_000,
                },
                None,
            )),
        ),
        ("Notes", [1100.0, -1100.0, 640.0, 520.0], None),
    ]
}

fn local([x, y, w, h]: [f64; 4]) -> [f32; 4] {
    [
        (x - ORIGIN.0) as f32,
        (y - ORIGIN.1) as f32,
        w as f32,
        h as f32,
    ]
}

pub(crate) fn view(state: &TwoDisplays, cx: &mut Cx<TwoDisplays>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let c = &p.k.c;
    let all = screens();
    let mut root = el()
        .key("two-displays")
        .size(SIZE.0, SIZE.1)
        .fill(rune::Color::BLACK);
    for screen in &all {
        let [x, y, w, h] = local(screen.frame);
        root = root.child(
            el().key(format!("display-{}", screen.key))
                .absolute(x, y)
                .size(w, h)
                .fill(c.bg_deep)
                .child(
                    row()
                        .key("menu-bar")
                        .absolute(0.0, 0.0)
                        .size(w, MENU_BAR)
                        .px(16.0)
                        .gap(18.0)
                        .center_y()
                        .fill(c.bg_base)
                        .children(
                            ["Hex", "File", "Edit", "View"]
                                .map(|word| text(word).font(p.k.ty.small_medium).color(p.muted())),
                        ),
                ),
        );
    }
    for (app, rect, outlined) in windows() {
        let [x, y, w, h] = local(rect);
        root = root.child(window(p, app, [x, y, w, h]));
        let Some((colour, title, look, progress)) = outlined else {
            continue;
        };
        let top = screens::at(&all, (rect[0] + rect[2] / 2.0, rect[1]))
            .map_or(0.0, |screen| screen.frame[1] + f64::from(MENU_BAR));
        let (at, label) = outline::rect((rect[0], rect[1], rect[2], rect[3]), top);
        let at = local(at.map(f64::from));
        let shown = Outline {
            colour,
            title: title.into(),
            size: (w, h),
            label,
            look,
            progress,
            origin: (0.0, 0.0),
        };
        root = root.child(
            el().key(format!("outline-{app}"))
                .absolute(at[0], at[1])
                .size(at[2], at[3])
                .child(outline::drawn(p, &shown, 0.0, 0.0)),
        );
    }
    // The person works on the M27Q: the HUD and the orb are there.
    let working = &all[1];
    let [wx, wy, ww, _] = local(working.frame);
    let hud_x = wx + (ww - HUD.0) / 2.0;
    root = root.child(
        el().key("hud")
            .absolute(hud_x, wy + MENU_BAR + 8.0)
            .size(HUD.0, HUD.1)
            .radius(HUD.1 / 2.0)
            .fill(c.bg_inverse),
    );
    let centre = working.frame[0] + working.frame[2] / 2.0;
    let anchor = place::default_anchor(working.visible, Some((centre, true)));
    let grow = place::grow(anchor, working.visible);
    let [ox, oy, ow, oh] = local(place::rect(anchor, grow).map(f64::from));
    root = root.child(
        el().key("orb-layer")
            .absolute(ox, oy)
            .size(ow, oh)
            .padding(orb::INSET)
            .child(orb::content(
                p,
                &state.board,
                &orb::Look {
                    grow,
                    still: true,
                    answer_key: fixture::ANSWER_KEY.into(),
                    ..Default::default()
                },
                orb::Actions::<TwoDisplays>::inert(),
            )),
    );
    for screen in &all {
        let [x, y, w, h] = local(screen.frame);
        let seams = screens::seams(screen, &all);
        root = root.child(
            el().key(format!("frame-on-{}", screen.key))
                .absolute(x, y)
                .size(w, h)
                .clip_rounded(0.0)
                .child(frame::glow_joined(p.k.dark, false, (w, h), &seams)),
        );
    }
    root
}

/// A made-up app window: a title bar and an empty body.
fn window(p: Paint, app: &str, [x, y, w, h]: [f32; 4]) -> El {
    let c = &p.k.c;
    el().key(format!("window-{app}"))
        .absolute(x, y)
        .size(w, h)
        .radius(10.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .clip_rounded(10.0)
        .child(
            row()
                .key("title-bar")
                .w_full()
                .h(30.0)
                .px(12.0)
                .center_y()
                .fill(c.surface_raised_base)
                .child(
                    text(app.to_owned())
                        .ml(54.0)
                        .font(p.k.ty.small_medium)
                        .color(p.muted()),
                ),
        )
        .a11y_hidden()
}
