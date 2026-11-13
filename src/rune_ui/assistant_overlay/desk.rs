//! A pretend desktop with every assistant overlay drawn on it, for the
//! playground and the render tests: the frame, the window outlines over
//! made-up windows, the rail top right, the island at the notch with its
//! card, the tab under it (a borrow, or Hands off), the stage, and the
//! hint, each placed by the rules the host places the panels by
//! ([`layout`]). The real overlays are separate panels (`host`); this draws
//! the same views together, so they can be looked at side by side.

use std::time::Instant;

use rune::{Cx, El, el, row, text};

use super::super::controls::Paint;
use super::ask::Reply;
use super::board::{Board, Bubble, Due};
use super::hint::Hint;
use super::listening::{self, Listening};
use super::outline::{self, Outline};
use super::quick::Held;
use super::{bubble, fixture, frame, hint, island, orb, place, stage};
use crate::assistant::Event;

/// The pretend desktop's size, in points.
pub(crate) const SIZE: (f32, f32) = (1280.0, 800.0);
const MENU_BAR: f32 = 24.0;
/// The pretend notch, idle: the menu bar's height.
const NOTCH: (f32, f32) = (180.0, MENU_BAR);

/// What the pretend desktop shows.
pub(crate) struct Desk {
    pub(crate) board: Board,
    pub(crate) bubble: Option<Bubble>,
    pub(crate) frame: bool,
    /// Breathe and pulse (the playground), or hold still (renders).
    pub(crate) breathe: bool,
    /// What the HUD says past itself while the Action shortcut is held.
    pub(crate) hint: Option<Hint>,
    /// The Action shortcut held for a quick turn: the island listens and
    /// the card under it shows the live words.
    pub(crate) held: Option<Held>,
    /// The orb's list is unfolded.
    pub(crate) expanded: bool,
    /// The Here answer is read aloud.
    pub(crate) speaking: bool,
    /// "Finish setting up the assistant first" instead of the bubble.
    pub(crate) setup: bool,
    /// Conversation mode listens for a follow-up past the HUD.
    pub(crate) listening: Option<Listening>,
    /// A typed answer to the question, while its field shows.
    pub(crate) reply: Option<Reply>,
    /// The result bubble unfolded (its key).
    pub(crate) open_result: Option<u64>,
    /// What the last press asked for, for the stage's caption.
    pub(crate) note: String,
    /// The display's size in points ([`SIZE`]: a 13-inch MacBook's).
    pub(crate) size: (f32, f32),
    /// The display has a notch the island grows from; else the island is
    /// a free capsule under the menu bar's centre.
    pub(crate) notch: bool,
    /// A task's window in front for a moment (a borrow), or the front just
    /// given back: the island says it, the tab under it the rest.
    pub(crate) tab: Option<frame::Tab>,
    /// The stage, with its panel (long answers, results).
    pub(crate) stage: Option<stage::Stage>,
}

impl Desk {
    pub(crate) fn empty() -> Self {
        Self {
            board: Board::default(),
            bubble: None,
            frame: false,
            breathe: true,
            hint: None,
            held: None,
            expanded: false,
            speaking: false,
            setup: false,
            listening: None,
            reply: None,
            open_result: None,
            note: String::new(),
            size: SIZE,
            notch: true,
            tab: None,
            stage: None,
        }
    }

    /// Everything the script shows at its end, still.
    pub(crate) fn settled() -> Self {
        Self {
            board: fixture::board(),
            bubble: fixture::bubble_at(f64::MAX),
            // The desk frame shows only while the agent has the controls
            // (a borrow, Hands off: `docs/ux/redesign.md` §3.5).
            frame: false,
            breathe: false,
            ..Self::empty()
        }
    }

    pub(crate) fn apply(&mut self, event: &Event) {
        let now = Instant::now();
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

    /// Return on the answer field: the answer goes to its job (here, the
    /// note says so, and the job runs on).
    fn send(&mut self) {
        let Some(reply) = self.reply.take() else {
            return;
        };
        let Some(text) = reply.answer() else {
            self.reply = Some(reply);
            return;
        };
        self.event(
            Event::Started {
                conversation: reply.id.clone(),
                origin: crate::assistant::Origin::Chat,
                question: text.clone(),
            },
            format!("Answered {}: {text}", reply.id),
        );
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
        let mut actions = orb::Actions::<Desk>::inert();
        let list = &mut actions.list;
        list.stop = Desk::stop;
        list.take_over = Desk::take_over;
        list.hand_back = Desk::hand_back;
        list.stop_all = Desk::stop_all;
        list.toggle = |desk| desk.expanded = !desk.expanded;
        list.dismiss = |desk, id| {
            desk.board.dismiss_question(id);
        };
        list.open_preview = |desk, task, session| {
            desk.note = format!("Would open {session} of {task} live");
        };
        list.defer = |desk, id| desk.note = format!("Would ask {id} later");
        list.reply.start = |desk, id| desk.reply = Some(Reply::new(id));
        list.reply.cancel = |desk| desk.reply = None;
        list.reply.send = Desk::send;
        list.reply.edit = |desk| &mut desk.reply.get_or_insert_with(Reply::default).edit;
        list.results.open = |desk, target| desk.note = format!("Would open {target}");
        list.results.retry = |desk, id| desk.note = format!("Would try {id} again");
        list.results.dismiss = |desk, key| {
            desk.board.pops.dismiss(key);
        };
        list.results.expand = |desk, key| {
            desk.open_result = (desk.open_result != Some(key)).then_some(key);
        };
        list.results.recall = |desk, id| {
            desk.open_result = desk.board.recall(id, Instant::now());
            desk.expanded = false;
        };
        actions.jobs.open =
            |desk, scope, _| desk.note = format!("Would show every picture of {scope:?}");
        actions.jobs.all = |desk| desk.note = "Would show every live picture".into();
        actions
    }
}

pub(crate) fn view(desk: &Desk, cx: &mut Cx<Desk>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let reduced = cx.system_reduced_motion();
    draw(p, desk, reduced)
}

/// Where everything on the desk goes, by the placement rules
/// (`docs/ux/redesign.md` §3.1): the island at the notch (or under the
/// menu bar's centre), the tab under it, the rail top right (flipped left
/// off an outlined window), the island's card under them clear of the
/// focus window's title bar, the rail, and every outline's label, and the
/// labels shortened beside any card still over them. Points on the desk.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    /// The island's capsule: x, y, width, height.
    pub(crate) island: [f32; 4],
    pub(crate) tab: Option<[f32; 4]>,
    pub(crate) card: Option<[f32; 4]>,
    pub(crate) stage: Option<[f32; 4]>,
    /// The rail's anchor, and what its cards cover.
    pub(crate) rail_at: (f64, f64),
    pub(crate) rail: [f32; 4],
    /// Each outline's label: whole, and its room beside the cards.
    pub(crate) labels: Vec<([f32; 4], Option<f32>)>,
    /// The focus window's title bar, when a Here task works in it.
    pub(crate) focus: Option<[f32; 4]>,
    /// How many floating cards the rail may show.
    pub(crate) rail_cards: usize,
}

fn area(a: [f64; 4]) -> [f32; 4] {
    a.map(|v| v as f32)
}

fn wide(a: [f32; 4]) -> [f64; 4] {
    a.map(f64::from)
}

/// The stage card's top that shows whatever it projects.
const STAGE_TOP_H: f32 = 200.0;

/// A window's title bar, kept clear when it's the focus window.
const TITLE_BAR: f64 = 30.0;

/// [`Layout`] for `desk`, its island saying `line`.
pub(crate) fn layout(desk: &Desk, line: Option<&island::Line>) -> Layout {
    let (w, h) = desk.size;
    let size = (f64::from(w), f64::from(h));
    let visible = [
        0.0,
        f64::from(MENU_BAR),
        size.0,
        size.1 - f64::from(MENU_BAR),
    ];
    let (top, _, bottom) = island::hang(desk.notch.then_some(MENU_BAR), MENU_BAR);
    let island_w = line.map_or(NOTCH.0, island::width);
    let island = [(w - island_w) / 2.0, top, island_w, bottom - top];
    let mut past = bottom + bubble::ISLAND_GAP;
    let tab = tab_of(desk).map(|tab| {
        let tab_w = frame::tab_width(&tab, tab.left(super::when::now_ms()));
        let at = [
            (w - tab_w) / 2.0,
            bottom + frame::TAB_GAP,
            tab_w,
            frame::TAB_H,
        ];
        past = at[1] + at[3] + bubble::ISLAND_GAP;
        at
    });
    let staging = desk
        .stage
        .as_ref()
        .is_some_and(|projected| projected.panel.is_some());
    let card_h = island_card(desk).map(|held| bubble::about_h(desk.bubble.as_ref(), held));
    let rail_cards = orb::rail_cards(card_h.is_some(), staging);
    // The outlined windows, their labels, and the focus window (the Here
    // task's).
    let mut windows: Vec<place::Area> = Vec::new();
    let mut labels: Vec<[f64; 4]> = Vec::new();
    let mut keep: Vec<place::Keep> = Vec::new();
    let mut focus = None;
    for item in desk.board.outlined() {
        let Some(rect) = fixture::frame(&item.window, size) else {
            continue;
        };
        windows.push([rect.0, rect.1, rect.2, rect.3]);
        let natural = outline::label_width(&shown(&item, rect), rect.2 as f32);
        let (whole, least) = outline::label_rects(rect, f64::from(MENU_BAR), natural);
        labels.push(whole);
        keep.push(place::Keep::Label { whole, least });
        if item.here {
            let bar = [rect.0, rect.1, rect.2, TITLE_BAR];
            keep.push(place::Keep::Hard(bar));
            focus = Some(area(bar));
        }
    }
    let height = place::rail_height(
        &desk.board,
        desk.expanded,
        f64::from(orb::SIZE.1),
        rail_cards,
    );
    let rail_at = place::clear_of(
        place::default_anchor(visible, None),
        visible,
        &windows,
        height,
    );
    let rail = place::footprint(rail_at, place::grow(rail_at, visible), height);
    keep.push(place::Keep::Hard(rail));
    // The stage under the island, moved aside off the rail.
    let projected = staging.then(|| {
        let (sw, sh) = stage::SIZE;
        let want = [
            (size.0 - f64::from(sw)) / 2.0,
            f64::from(past - stage::INSET),
            f64::from(sw),
            f64::from(sh.min(h - past)),
        ];
        area(place::beside_rail(
            want,
            f64::from(stage::INSET),
            rail,
            visible,
        ))
    });
    if let Some(tab) = tab {
        keep.push(place::Keep::Hard(wide(tab)));
    }
    let card = card_h.map(|card_h| {
        let (cw, ch) = (
            f64::from(bubble::SIZE.0),
            f64::from(card_h + 2.0 * bubble::INSET),
        );
        let want = (
            (size.0 - cw) / 2.0,
            f64::from(past) - f64::from(bubble::INSET),
        );
        let (x, y) = place::clear_spot(want, (cw, ch), visible, &keep);
        area([x, y, cw, ch])
    });
    // Labels shorten beside the rail, the tab, the island's card, and the
    // stage's card (its top, which is there whatever it shows).
    let mut cards: Vec<place::Area> = vec![rail];
    cards.extend(tab.map(wide));
    cards.extend(card.map(wide));
    cards.extend(projected.map(|[x, y, w, _]| {
        let inset = stage::INSET;
        wide([x + inset, y + inset, w - 2.0 * inset, STAGE_TOP_H])
    }));
    let labels = labels
        .into_iter()
        .map(|whole| {
            let room = place::label_room(whole, &cards).map(|room| room as f32);
            (area(whole), room)
        })
        .collect();
    Layout {
        island,
        tab,
        card,
        stage: projected,
        rail_at,
        rail: area(rail),
        labels,
        focus,
        rail_cards,
    }
}

/// Whether the island shows a card under it, and whether it's the live
/// words (`Some(true)`).
fn island_card(desk: &Desk) -> Option<bool> {
    if desk.listening.is_some() || desk.hint.is_some() {
        return None;
    }
    if desk.held.is_some() {
        return Some(true);
    }
    (desk.setup || desk.bubble.is_some()).then_some(false)
}

/// The tab under the island: a borrow, else real input (Hands off has one
/// surface, the tab; no pill of its own).
fn tab_of(desk: &Desk) -> Option<frame::Tab> {
    if let Some(tab) = &desk.tab {
        return Some(tab.clone());
    }
    let task = desk.board.hands_off()?;
    let window = task.windows.first().cloned();
    Some(frame::Tab::Hands {
        app: window.as_ref().map(|window| window.app.clone()),
        window,
    })
}

fn shown(item: &super::board::Outlined, rect: (f64, f64, f64, f64)) -> Outline {
    Outline {
        colour: item.colour,
        title: item.title.clone(),
        size: (rect.2 as f32, rect.3 as f32),
        label: outline::rect(rect, f64::from(MENU_BAR)).1,
        look: item.look,
        progress: item.progress,
        step: item.step.clone(),
        here: item.here,
        origin: (0.0, 0.0),
        room: None,
    }
}

/// The desktop at its size ([`SIZE`] unless set).
pub(crate) fn draw(p: Paint, desk: &Desk, reduced: bool) -> El {
    let c = &p.k.c;
    let still = !desk.breathe || reduced;
    let (dw, dh) = desk.size;
    let size = (f64::from(dw), f64::from(dh));
    let line = island::line_with(
        desk.held.as_ref(),
        desk.tab.as_ref(),
        desk.bubble.as_ref().filter(|_| !desk.setup),
    );
    let at = layout(desk, line.as_ref());
    let mut root = el()
        .key("desk")
        .size(dw, dh)
        .fill(c.bg_deep)
        .clip_rounded(0.0)
        .child(
            row()
                .key("menu-bar")
                .absolute(0.0, 0.0)
                .size(dw, MENU_BAR)
                .px(16.0)
                .gap(18.0)
                .center_y()
                .fill(c.bg_base)
                .border_sides([0.0, 0.0, p.hair, 0.0], c.border_base)
                .children(
                    ["Agent-Desk", "File", "Edit", "View"]
                        .map(|word| text(word).font(p.k.ty.small_medium).color(p.muted())),
                ),
        );
    for (app, id, _) in fixture::WINDOWS {
        if let Some(rect) = fixture::frame(&fixture::window(id), size) {
            root = root.child(window(p, app, rect));
        }
    }
    let mut rooms = at.labels.iter();
    for item in desk.board.outlined() {
        let Some(rect) = fixture::frame(&item.window, size) else {
            continue;
        };
        let (panel, _) = outline::rect(rect, f64::from(MENU_BAR));
        let drawn = Outline {
            room: rooms.next().and_then(|(_, room)| *room),
            ..shown(&item, rect)
        };
        root = root.child(
            el().key(format!("outline-{}", item.task))
                .absolute(panel[0], panel[1])
                .size(panel[2], panel[3])
                .child(outline::drawn(p, &drawn, 0.0, 0.0)),
        );
    }
    // The island: the notch grown down with the conversation's one line,
    // black in both appearances (`island`); the notch alone when idle.
    // Drawn last, over the frame's glow: it is the notch, the topmost thing.
    let [ix, iy, iw, _] = at.island;
    let island_el = match &line {
        Some(line) => el().key("hud").absolute(ix, iy).child(island::capsule(
            p,
            line,
            iw,
            if desk.notch { MENU_BAR } else { 0.0 },
        )),
        None if desk.notch => el()
            .key("hud")
            .absolute((dw - NOTCH.0) / 2.0, 0.0)
            .child(island::notch(NOTCH)),
        None => el().key("hud"),
    };
    let tab = tab_of(desk);
    if let (Some(tab), Some([_, ty, _, _])) = (&tab, at.tab) {
        let left = tab.left(super::when::now_ms());
        root = root.child(
            row()
                .key("tab-layer")
                .absolute(0.0, ty)
                .w(dw)
                .justify_center()
                .child(frame::tab_view(p, tab, left)),
        );
    }
    if let (Some(projected), Some([sx, sy, sw, sh])) = (&desk.stage, at.stage) {
        root = root.child(
            el().key("stage-layer")
                .absolute(sx, sy)
                .size(sw, sh)
                .child(stage::drawn(p, projected)),
        );
    }
    let under = at
        .tab
        .map_or(at.island[1] + at.island[3], |tab| tab[1] + tab[3])
        + 8.0;
    let [cx, cy, _, _] = at.card.unwrap_or_default();
    if let Some(state) = &desk.listening {
        let (w, h) = listening::SIZE;
        root = root.child(
            el().key("listening-layer")
                .absolute((dw - w) / 2.0, under)
                .size(w, h)
                .center()
                .child(listening::drawn(p, state, still)),
        );
    } else if let Some(said) = &desk.hint {
        let (w, h) = hint::SIZE;
        root = root.child(
            el().key("hint-layer")
                .absolute((dw - w) / 2.0, under)
                .size(w, h)
                .center()
                .child(hint::drawn(p, said)),
        );
    } else if let Some(held) = &desk.held {
        let (w, _) = bubble::SIZE;
        root = root.child(
            el().key("bubble")
                .absolute(cx, cy)
                .w(w)
                .padding(bubble::INSET)
                .child(island::held_card(p, held, fixture::ANSWER_KEY)),
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
            settings: |desk: &mut Desk| desk.note = "Would open Assistant settings".into(),
            retry: |desk: &mut Desk, _, asked| desk.note = format!("Would ask again: {asked}"),
            take_project: |_: &mut Desk, _| {},
        };
        let card = match &desk.bubble {
            _ if desk.setup => bubble::setup_card(p, actions),
            Some(said) => bubble::card(
                p,
                said,
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
                .absolute(cx, cy)
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
    let grow = place::grow(at.rail_at, visible);
    let [x, y, w, h] = place::fitted(at.rail_at, grow, visible);
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
                    reply: desk.reply.clone(),
                    open_result: desk.open_result,
                    room: Some(h),
                    cards: Some(at.rail_cards),
                    ..Default::default()
                },
                Desk::actions(),
            )),
    );
    if desk.frame || tab.as_ref().is_some_and(frame::Tab::in_front) {
        root = root.child(
            el().key("frame-layer")
                .absolute(0.0, 0.0)
                .size(dw, dh)
                .child(frame::glow(p.k.dark)),
        );
    }
    root = root.child(island_el);
    if !desk.note.is_empty() {
        root = root.child(
            text(desk.note.clone())
                .key("note")
                .absolute(24.0, dh - 36.0)
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

#[cfg(test)]
#[path = "desk_tests.rs"]
mod tests;
