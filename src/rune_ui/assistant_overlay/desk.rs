//! A pretend desktop with every assistant overlay drawn on it, for the
//! playground and the render tests: the frame, the window outlines over
//! made-up windows, the task stack, and the quick question's bubble below
//! a stand-in HUD. The real overlays are separate panels (`host`); this
//! draws the same views together, so they can be looked at side by side
//! without Agent mode.

use std::time::Instant;

use rune::{Cx, El, el, row, text};

use super::super::controls::Paint;
use super::board::{Board, Bubble, Due};
use super::fixture;
use super::outline::{self, Outline};
use super::{bubble, frame, hint, stack};
use crate::assistant::Event;

/// The pretend desktop's size, in points.
pub(crate) const SIZE: (f32, f32) = (1280.0, 800.0);
const MENU_BAR: f32 = 24.0;
const HUD: (f32, f32) = (132.0, 34.0);

/// What the pretend desktop shows.
pub(crate) struct Desk {
    pub(crate) board: Board,
    pub(crate) bubble: Option<Bubble>,
    pub(crate) frame: bool,
    /// The frame breathes (the playground), or holds still (renders).
    pub(crate) breathe: bool,
    /// The Action shortcut is held in Agent mode: "New task" past the HUD.
    pub(crate) holding: bool,
    /// What the last press asked for, for the stage's caption.
    pub(crate) note: String,
}

impl Desk {
    pub(crate) fn empty() -> Self {
        Self {
            board: Board::default(),
            bubble: None,
            frame: false,
            breathe: true,
            holding: false,
            note: String::new(),
        }
    }

    /// Everything the script shows at its end, still.
    pub(crate) fn settled() -> Self {
        Self {
            board: fixture::board(),
            bubble: fixture::bubble_at(f64::MAX),
            frame: true,
            breathe: false,
            holding: false,
            note: String::new(),
        }
    }

    pub(crate) fn apply(&mut self, event: &Event) {
        let now = Instant::now();
        if let Event::ModeChanged { on } = event {
            self.frame = *on;
            return;
        }
        self.board.apply(event, None, now);
        Bubble::apply(&mut self.bubble, event, now);
    }

    /// Finished tasks and an answered bubble leave on time.
    pub(crate) fn tick(&mut self, _clock: f64, tx: &mut rune::Tx) -> bool {
        let now = Instant::now();
        let mut changed = self.board.expire(now);
        if let Some(bubble) = self.bubble.as_mut() {
            match bubble.due(now, false) {
                Due::Leave => {
                    self.bubble = None;
                    changed = true;
                }
                Due::At(at) => tx.wake_after(at.saturating_duration_since(now)),
                Due::Stay => {}
            }
        }
        if let Some(at) = self.board.next_expiry() {
            tx.wake_after(at.saturating_duration_since(now));
        }
        changed
    }

    fn stop(&mut self, id: &str) {
        self.apply(&Event::Cancelled {
            conversation: id.into(),
        });
        self.note = format!("Stopped {id}");
    }

    fn open(&mut self, id: &str) {
        self.note = format!("Would open “{id}” in Chats");
    }
}

pub(crate) fn view(desk: &Desk, cx: &mut Cx<Desk>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let reduced = cx.system_reduced_motion();
    draw(p, desk, reduced)
}

/// The desktop at [`SIZE`].
pub(crate) fn draw(p: Paint, desk: &Desk, reduced: bool) -> El {
    let c = &p.k.c;
    let mut root = el()
        .key("desk")
        .size(SIZE.0, SIZE.1)
        .fill(c.bg_deep)
        .clip_rounded(0.0)
        .child(
            row()
                .key("menu-bar")
                .absolute(0.0, 0.0)
                .size(SIZE.0, MENU_BAR)
                .px(16.0)
                .gap(18.0)
                .center_y()
                .fill(c.bg_base)
                .border_sides([0.0, 0.0, p.hair, 0.0], c.border_base)
                .children(
                    ["Hex", "File", "Edit", "View"]
                        .map(|word| text(word).font(p.k.ty.small_medium).color(p.muted())),
                ),
        );
    let size = (f64::from(SIZE.0), f64::from(SIZE.1));
    for (app, id, _) in fixture::WINDOWS {
        if let Some(rect) = fixture::frame(&fixture::window(id), size) {
            root = root.child(window(p, app, rect));
        }
    }
    for item in desk.board.outlined() {
        let Some(rect) = fixture::frame(&item.window, size) else {
            continue;
        };
        let (at, label) = outline::rect(rect, f64::from(MENU_BAR));
        let shown = Outline {
            colour: item.colour,
            title: item.title.clone(),
            size: (rect.2 as f32, rect.3 as f32),
            label,
        };
        root = root.child(
            el().key(format!("outline-{}", item.task))
                .absolute(at[0], at[1])
                .size(at[2], at[3])
                .child(outline::drawn(p, &shown, 0.0, 0.0)),
        );
    }
    // The stand-in HUD, centred under the menu bar.
    let hud_x = (SIZE.0 - HUD.0) / 2.0;
    let hud_y = MENU_BAR + 8.0;
    root = root.child(
        el().key("hud")
            .absolute(hud_x, hud_y)
            .size(HUD.0, HUD.1)
            .radius(HUD.1 / 2.0)
            .fill(c.bg_inverse)
            .a11y_hidden(),
    );
    if desk.holding {
        let (w, h) = hint::SIZE;
        root = root.child(
            el().key("hint-layer")
                .absolute((SIZE.0 - w) / 2.0, hud_y + HUD.1 + 4.0)
                .size(w, h)
                .center()
                .child(hint::pill(p)),
        );
    } else if let Some(bubble) = &desk.bubble {
        let (w, _) = bubble::SIZE;
        root = root.child(
            el().key("bubble")
                .absolute((SIZE.0 - w) / 2.0, hud_y + HUD.1 + 6.0)
                .w(w)
                .padding(bubble::INSET)
                .child(bubble::card(
                    p,
                    bubble,
                    bubble::Actions {
                        dismiss: |desk: &mut Desk| desk.bubble = None,
                        open: Desk::open,
                    },
                )),
        );
    }
    let tasks = desk.board.stack();
    if !tasks.is_empty() {
        let (w, h) = stack::SIZE;
        root = root.child(
            el().key("stack")
                .absolute(
                    SIZE.0 - w - (stack::MARGIN - stack::INSET),
                    SIZE.1 - h - 4.0,
                )
                .size(w, h)
                .padding(stack::INSET)
                .child(stack::plates(
                    p,
                    &tasks,
                    stack::Actions {
                        stop: Desk::stop,
                        open: Desk::open,
                    },
                )),
        );
    }
    if desk.frame {
        root = root.child(
            el().key("frame-layer")
                .absolute(0.0, 0.0)
                .size(SIZE.0, SIZE.1)
                .child(frame::glow(p.k.dark, desk.breathe && !reduced)),
        );
    }
    if !desk.note.is_empty() {
        root = root.child(
            text(desk.note.clone())
                .key("note")
                .absolute(24.0, SIZE.1 - 36.0)
                .font(p.k.ty.small)
                .color(p.faint()),
        );
    }
    root
}

/// A made-up app window: a title bar and an empty body.
fn window(p: Paint, app: &str, (x, y, w, h): (f64, f64, f64, f64)) -> El {
    let c = &p.k.c;
    el().key(format!("window-{app}"))
        .absolute(x as f32, y as f32)
        .size(w as f32, h as f32)
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
                .gap(6.0)
                .center_y()
                .fill(c.surface_raised_base)
                .children((0..3).map(|i| {
                    el().key(format!("light-{i}"))
                        .size(10.0, 10.0)
                        .radius(5.0)
                        .fill(c.border_strong)
                }))
                .child(
                    text(app.to_owned())
                        .ml(10.0)
                        .font(p.k.ty.small_medium)
                        .color(p.muted()),
                ),
        )
        .a11y_hidden()
}
