//! A pretend desktop with every assistant overlay drawn on it, for the
//! playground and the render tests: the frame, the window outlines over
//! made-up windows, the orb with its bubbles or list beside a stand-in HUD,
//! the Here question's bubble below the HUD, and the hint or the Hands off
//! pill past it. The real overlays are separate panels (`host`); this draws
//! the same views together, so they can be looked at side by side.

use std::time::Instant;

use rune::{Cx, El, el, row, text};

use super::super::controls::Paint;
use super::board::{Board, Bubble, Due};
use super::hint::Hint;
use super::listening::{self, Listening};
use super::outline::{self, Outline};
use super::{bubble, fixture, frame, hands, hint, orb, place, stack};
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
    /// Breathe and pulse (the playground), or hold still (renders).
    pub(crate) breathe: bool,
    /// What the HUD says past itself while the Action shortcut is held.
    pub(crate) hint: Option<Hint>,
    /// The orb's list is unfolded.
    pub(crate) expanded: bool,
    /// The Here answer is read aloud.
    pub(crate) speaking: bool,
    /// "Finish setting up the assistant first" instead of the bubble.
    pub(crate) setup: bool,
    /// Conversation mode listens for a follow-up past the HUD.
    pub(crate) listening: Option<Listening>,
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
            hint: None,
            expanded: false,
            speaking: false,
            setup: false,
            listening: None,
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
            ..Self::empty()
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

    /// Finished tasks, bubbles, and an answered Here bubble leave on time.
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

    fn event(&mut self, event: Event, note: String) {
        self.apply(&event);
        self.note = note;
    }

    fn stop(&mut self, id: &str) {
        self.event(
            Event::Cancelled {
                conversation: id.into(),
            },
            format!("Stopped {id}"),
        );
    }

    fn open(&mut self, id: &str) {
        self.note = format!("Would open “{id}” in Chats");
    }

    fn take_over(&mut self, id: &str) {
        self.event(
            Event::Paused {
                conversation: id.into(),
            },
            format!("You have {id}"),
        );
    }

    fn hand_back(&mut self, id: &str) {
        self.event(
            Event::Resumed {
                conversation: id.into(),
            },
            format!("Handed {id} back"),
        );
    }

    fn stop_all(&mut self) {
        let live: Vec<String> = self
            .board
            .tasks
            .iter()
            .filter(|task| task.phase.live())
            .map(|task| task.id.clone())
            .collect();
        for id in live {
            self.stop(&id);
        }
        self.note = "Stopped all".into();
    }

    fn actions() -> orb::Actions<Desk> {
        orb::Actions {
            list: stack::Actions {
                stop: Desk::stop,
                open: Desk::open,
                take_over: Desk::take_over,
                hand_back: Desk::hand_back,
                stop_all: Desk::stop_all,
                toggle: |desk| desk.expanded = !desk.expanded,
                dismiss: |desk, id| {
                    desk.board.dismiss_question(id);
                },
                hover: |_, _, _| {},
                open_preview: |desk, task, session| {
                    desk.note = format!("Would open {session} of {task} live");
                },
                defer: |desk, id| desk.note = format!("Would ask {id} later"),
            },
            hover: |_, _| {},
            drag: |_, _| {},
        }
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
    let still = !desk.breathe || reduced;
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
            look: item.look,
            progress: item.progress,
            origin: (0.0, 0.0),
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
    let mut past = hud_y + HUD.1 + 4.0;
    if desk.board.hands_off().is_some() {
        let (w, h) = hands::SIZE;
        root = root.child(
            el().key("hands-layer")
                .absolute((SIZE.0 - w) / 2.0, past)
                .size(w, h)
                .center()
                .child(hands::pill(p)),
        );
        past += h;
    }
    if let Some(state) = &desk.listening {
        let (w, h) = listening::SIZE;
        root = root.child(
            el().key("listening-layer")
                .absolute((SIZE.0 - w) / 2.0, past)
                .size(w, h)
                .center()
                .child(listening::drawn(p, state, still)),
        );
    } else if let Some(shown) = &desk.hint {
        let (w, h) = hint::SIZE;
        root = root.child(
            el().key("hint-layer")
                .absolute((SIZE.0 - w) / 2.0, past)
                .size(w, h)
                .center()
                .child(hint::drawn(p, shown)),
        );
    } else if desk.setup || desk.bubble.is_some() {
        let (w, _) = bubble::SIZE;
        let actions = bubble::Actions {
            dismiss: |desk: &mut Desk| {
                desk.bubble = None;
                desk.setup = false;
            },
            open: Desk::open,
            setup: |desk: &mut Desk| desk.note = "Would open the assistant's setup".into(),
        };
        let card = match &desk.bubble {
            _ if desk.setup => bubble::setup_card(p, actions),
            Some(shown) => bubble::card(
                p,
                shown,
                bubble::Voice {
                    speaking: desk.speaking,
                    still,
                    level: None,
                },
                fixture::ANSWER_KEY,
                actions,
            ),
            None => el(),
        };
        root = root.child(
            el().key("bubble")
                .absolute((SIZE.0 - w) / 2.0, past + 2.0)
                .w(w)
                .padding(bubble::INSET)
                .child(card),
        );
    }
    let visible = [
        0.0,
        f64::from(MENU_BAR),
        size.0,
        size.1 - f64::from(MENU_BAR),
    ];
    let anchor = place::default_anchor(visible, Some((size.0 / 2.0, true)));
    let grow = place::grow(anchor, visible);
    let [x, y, w, h] = place::rect(anchor, grow);
    root = root.child(
        el().key("orb-layer")
            .absolute(x, y)
            .size(w, h)
            .padding(orb::INSET)
            .child(orb::content(
                p,
                &desk.board,
                &orb::Look {
                    open: desk.expanded,
                    grow,
                    still,
                    answer_key: fixture::ANSWER_KEY.into(),
                    voice: None,
                    hovered: None,
                },
                Desk::actions(),
            )),
    );
    if desk.frame {
        root = root.child(
            el().key("frame-layer")
                .absolute(0.0, 0.0)
                .size(SIZE.0, SIZE.1)
                .child(frame::glow(p.k.dark, !still)),
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
