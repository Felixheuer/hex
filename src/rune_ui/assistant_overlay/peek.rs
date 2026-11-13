//! The live overview in the rail: every browser and window of a job and
//! its helpers (or, from "All live" at the rail's head, of every job and
//! the desk), small and live, two to a line, each with its site and a
//! "● Live" dot or how long ago its last frame came. It takes the job
//! rows' place in the rail, so it covers nothing the rail doesn't (the
//! rail keeps clear of the focus window's title bar), and scrolls inside
//! the panel's real room.
//!
//! It opens on a press of "All N live" (and then has the keyboard:
//! Escape and a click outside close it) or when the pointer rests on it
//! ([`INTENT`]); it closes itself once the pointer has left the rail for
//! [`LEAVE`], and on ×. A press on a picture opens it large (the enlarged
//! card, as big as the display allows, with Take over); "Large" opens the
//! wall. Only the pictures in view stream (`live::seen`), six frames a
//! second; the rest rest until scrolled to.

use std::time::{Duration, Instant};

use rune::{El, col, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::super::icons::{GLOBE, PANEL};
use super::board::Board;
use super::minis::Desk;
use super::pictures::Picture;
use super::wall::{self, Group, Scope, Tile};
use super::{jobs, live, look, orb, palette};
use crate::desktop::i18n::{tr, tr_fill};

/// How long the pointer rests on "All N live" before the overview opens.
pub(crate) const INTENT: Duration = Duration::from_millis(450);
/// How long the pointer may stay off the rail before it closes.
pub(crate) const LEAVE: Duration = Duration::from_millis(1500);
/// Pictures on a line.
pub(crate) const COLS: usize = 2;
const PAD: f32 = 11.0;
const GAP: f32 = 8.0;
/// A picture: two fill the rail's width, in a page's shape (16:10).
pub(crate) const TILE_W: f32 = (jobs::CARD_W - 2.0 * PAD - GAP) / COLS as f32;
pub(crate) const TILE_H: f32 = TILE_W / 1.6;
/// The overview's header.
const HEAD_H: f32 = 28.0;

/// What the overview asks of the host.
pub(crate) struct Actions<S> {
    /// × (and Escape): it closes.
    pub(crate) close: fn(&mut S),
    /// "Large": the live wall of the same pictures.
    pub(crate) wall: fn(&mut S, &Scope),
    /// Picture (task, session) came into view (`true`) or left it.
    pub(crate) seen: fn(&mut S, &str, &str, bool),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

impl<S> Actions<S> {
    pub(crate) fn inert() -> Self {
        Self {
            close: |_| {},
            wall: |_, _| {},
            seen: |_, _, _, _| {},
        }
    }
}

/// The overview's state in the rail.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Peek {
    pub(crate) scope: Scope,
    /// Opened by a press: it has the keyboard (else the pointer opened it).
    pub(crate) pressed: bool,
    /// When the pointer left the rail, while it's off it.
    pub(crate) left: Option<Instant>,
}

impl Peek {
    pub(crate) fn new(scope: Scope, pressed: bool) -> Self {
        Self {
            scope,
            pressed,
            left: None,
        }
    }

    /// Whether the pointer has been off the rail long enough at `now`.
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.left.is_some_and(|left| now >= left + LEAVE)
    }
}

/// The tasks whose pictures the overview of `scope` shows.
pub(crate) fn tasks(board: &Board, desk: &Desk, scope: &Scope) -> Vec<String> {
    wall::tasks(board, desk, scope)
}

/// The overview of `scope` on `board` and `desk`, at most `max_h` tall;
/// a picture's press is `open` (task, session).
#[allow(clippy::too_many_arguments)]
pub(crate) fn view<S: 'static>(
    p: Paint,
    board: &Board,
    desk: &Desk,
    scope: &Scope,
    max_h: f32,
    still: bool,
    open: fn(&mut S, &str, &str),
    actions: Actions<S>,
) -> El {
    let k = p.k;
    let groups = wall::groups(board, desk, scope);
    let count: usize = groups.iter().map(|group| group.tiles.len()).sum();
    let lead = match scope {
        Scope::Job(lead) => board.tasks.iter().find(|task| &task.id == lead),
        Scope::All => None,
    };
    let marker = match lead {
        Some(task) => orb::marker(p, task, palette::task(task.colour, k.dark), 9.0),
        None => icon(&wall::GRID, 13.0).color(p.muted()).a11y_hidden(),
    };
    let title = match lead {
        Some(task) => task.title.clone(),
        None => tr("Everything live").to_owned(),
    };
    let (close, to_wall, seen) = (actions.close, actions.wall, actions.seen);
    let wall_scope = scope.clone();
    let head = row()
        .key("peek-head")
        .w_full()
        .h(HEAD_H)
        .gap(7.0)
        .center_y()
        .child(marker)
        .child(
            text(title)
                .key("peek-title")
                .role("heading")
                .truncate()
                .font(look::label(p))
                .color(p.text())
                .min_w(0.0)
                .grow(1.0),
        )
        .child(
            text(tr_fill("{} live", &count.to_string()))
                .key("peek-count")
                .font(look::meta(p))
                .color(p.faint()),
        )
        .child(
            IconButton::new(&wall::GRID)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Show all large"))
                .on_press(move |s: &mut S| to_wall(s, &wall_scope))
                .build(k)
                .key("peek-wall"),
        )
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Close"))
                .on_press(move |s: &mut S| close(s))
                .build(k)
                .key("peek-close"),
        );
    let headed = groups.len() > 1 || *scope == Scope::All;
    let mut lines: Vec<El> = Vec::new();
    for (at, group) in groups.iter().enumerate() {
        if headed {
            lines.push(group_head(p, group, at));
        }
        for (line, tiles) in group.tiles.chunks(COLS).enumerate() {
            lines.push(
                row()
                    .key(format!("peek-line-{at}-{line}"))
                    .gap(GAP)
                    .children(tiles.iter().map(|t| {
                        let session = session_of(t);
                        jobs::seen_as(
                            tile(p, t, lead.map(|task| task.id.as_str()), still, open),
                            &t.owner,
                            &session,
                            seen,
                        )
                    })),
            );
        }
    }
    if lines.is_empty() {
        lines.push(
            text(tr("Nothing live right now"))
                .key("peek-empty")
                .font(look::meta(p))
                .color(p.faint())
                .py(12.0),
        );
    }
    let grid = super::wheel::scroller::<S>(
        col(),
        super::wheel::Surface::Orb,
        "peek-grid",
        super::wheel::Axis::Y,
    )
    .key("peek-grid")
    .w_full()
    .min_h(0.0)
    .shrink()
    .gap(GAP)
    .scroll(rune::ScrollSpec {
        stick_bottom: false,
    })
    .scroll_edge_fade(14.0)
    // The wheel answers between the pictures too.
    .hit()
    .role("list")
    .label(tr("Live windows"))
    .children(lines);
    look::material(
        p,
        col()
            .key("peek")
            .w(jobs::CARD_W)
            .max_h(max_h)
            .min_h(0.0)
            .padding(PAD)
            .pt(6.0)
            .gap(6.0),
    )
    .role("dialog")
    .label(tr("Live overview"))
    // Presses between the pictures stay here (and don't unfold the list).
    .hit()
    .on_press(|_: &mut S| {})
    .on_key(move |s: &mut S, key: &rune::KeyEvent, _| {
        if key.key == rune::Key::Escape {
            close(s);
            true
        } else {
            false
        }
    })
    .child(head)
    .child(grid)
}

/// A group's line in the overview of everything: its dot, title, count.
fn group_head(p: Paint, group: &Group, at: usize) -> El {
    let (dot, title) = match group.task {
        Some(task) => (
            Some(orb::dot(task, palette::task(task.colour, p.k.dark), 7.0)),
            task.title.clone(),
        ),
        None => (None, tr("Your desk").to_owned()),
    };
    row()
        .key(format!("peek-group-{at}"))
        .w_full()
        .h(18.0)
        .gap(6.0)
        .center_y()
        .role("heading")
        .children(dot)
        .child(
            text(title)
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0)
                .grow(1.0),
        )
        .child(
            text(group.tiles.len().to_string())
                .font(look::meta(p))
                .color(p.faint()),
        )
}

/// A picture's session (a browser's, or `app:<number>`).
pub(crate) fn session_of(tile: &Tile) -> String {
    match &tile.picture {
        Picture::Browser(preview) => preview.session.clone(),
        Picture::App(preview) => preview.session(),
    }
}

/// One picture: live and small with its liveness, or an opaque card with
/// its name while none came; under it its site (and a helper's dot).
fn tile<S: 'static>(
    p: Paint,
    tile: &Tile,
    lead: Option<&str>,
    still: bool,
    open: fn(&mut S, &str, &str),
) -> El {
    let k = p.k;
    let c = &k.c;
    let session = session_of(tile);
    let (name, title, pixels, liveness, glyph) = match &tile.picture {
        Picture::Browser(preview) => {
            let page = preview.page();
            let name = live::host_of(&page.url)
                .or_else(|| (!preview.name.is_empty()).then(|| preview.name.clone()))
                .unwrap_or_else(|| tr("Browser").to_owned());
            let title = if page.title.is_empty() {
                name.clone()
            } else {
                page.title.clone()
            };
            let liveness = look::liveness(
                preview.clock.age(preview.has_frame()),
                page.failed.is_some(),
            );
            (name, title, &preview.pixels, liveness, &GLOBE)
        }
        Picture::App(preview) => {
            let name = preview.window.app.clone();
            let liveness = look::liveness(preview.clock.age(preview.has_frame()), preview.failed());
            (name.clone(), name, &preview.pixels, liveness, &PANEL)
        }
    };
    let picture = if pixels.size() != (0, 0) {
        el().size(TILE_W, TILE_H)
            .radius(8.0)
            .fill(c.layer_02)
            .border(p.hair, k.alpha(c.text_base, 0.14))
            .pixels_fit(pixels, rune::SurfaceFit::Fill)
    } else {
        look::no_picture(p, glyph, &title, liveness, (TILE_W, TILE_H))
    };
    let framed = el()
        .size(TILE_W, TILE_H)
        .child(picture)
        .children(look::live_badge(p, liveness, still).map(|badge| badge.absolute(5.0, 5.0)));
    let helper = tile
        .task
        .filter(|task| lead.is_some_and(|lead| task.id != lead))
        .map(|task| orb::dot(task, palette::task(task.colour, k.dark), 6.0));
    let caption = row()
        .w(TILE_W)
        .h(16.0)
        .gap(5.0)
        .center_y()
        .children(helper)
        .child(
            text(name.clone())
                .key("peek-name")
                .truncate()
                .font(look::meta(p))
                .color(p.muted())
                .min_w(0.0),
        );
    let (owner, at) = (tile.owner.clone(), session.clone());
    k.interactive(
        col()
            .key(format!("peek-tile-{session}"))
            .w(TILE_W)
            .gap(4.0)
            .child(framed)
            .child(caption)
            .role("button")
            .label(tr_fill("Open {} live", &name))
            .hit()
            .on_press(move |s: &mut S| open(s, &owner, &at)),
    )
}
