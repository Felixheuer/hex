//! The live wall: every browser and app window of a job and its helpers,
//! live at once ("Alle 20 live"). Twenty helpers each with a browser are
//! twenty small pictures in a grid sized to fill the panel, each captioned
//! with its helper and site; a press on one opens it large (the enlarged
//! card, with Take over), and × or Escape closes the wall.
//!
//! The pictures stream only while the wall shows: opening it puts its jobs
//! on `live::wall` and `app_live::wall` (two frames a second, at full size
//! so a tile stays sharp); closing it takes them off again.
//!
//! The rules are plain functions ([`tasks`], [`grid`]); the host
//! adds the panel and the streams (`host::wall`).

use rune::{Cx, El, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::app_live::{self, AppPreview};
use super::board::{Board, Phase, Task};
use super::live::{self, Preview};
use super::{orb, palette};
use crate::desktop::i18n::{tr, tr_fill};

/// The panel.
pub(crate) const SIZE: (f32, f32) = (1180.0, 780.0);
/// Padding around the grid, and the header above it.
const PAD: f32 = 20.0;
const HEADER_H: f32 = 40.0;
/// Space between tiles, and a tile's caption under its picture.
pub(crate) const GAP: f32 = 10.0;
pub(crate) const CAPTION_H: f32 = 18.0;
/// A browser page's shape (16:10), for tiles before their first frame.
const ASPECT: f32 = 1.6;

/// What the wall's buttons ask of the host.
pub(crate) struct Actions<S> {
    pub(crate) close: fn(&mut S),
    /// A tile: open that browser or window large (task, session).
    pub(crate) open: fn(&mut S, &str, &str),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The wall's state.
pub(crate) struct Wall {
    /// The job whose team it shows; `None` while closed.
    pub(crate) lead: Option<String>,
    /// The orb's board as it last said, for the team and its titles.
    pub(crate) board: Board,
    pub(crate) actions: Actions<Wall>,
}

impl Wall {
    pub(crate) fn new(actions: Actions<Wall>) -> Self {
        Self {
            lead: None,
            board: Board::default(),
            actions,
        }
    }
}

/// `lead` and its helpers on `board`, lead first, then its helpers oldest
/// first.
pub(crate) fn team<'a>(board: &'a Board, lead: &str) -> Vec<&'a Task> {
    let mut team: Vec<&Task> = board.tasks.iter().filter(|t| t.id == lead).collect();
    team.extend(
        board
            .tasks
            .iter()
            .filter(|task| task.lead.as_deref() == Some(lead)),
    );
    team
}

/// The task ids [`team`] gives, for the streams.
pub(crate) fn tasks(board: &Board, lead: &str) -> Vec<String> {
    team(board, lead)
        .into_iter()
        .map(|task| task.id.clone())
        .collect()
}

/// A tile grid: columns, rows, and each tile's picture size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) tile: (f32, f32),
}

/// The grid that shows `n` pictures of `aspect` (width / height) the
/// largest inside `room`, each with a caption under it.
pub(crate) fn grid(n: usize, room: (f32, f32), aspect: f32) -> Grid {
    let n = n.max(1);
    let mut best = Grid {
        cols: 1,
        rows: n,
        tile: (0.0, 0.0),
    };
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let width = (room.0 - GAP * (cols - 1) as f32) / cols as f32;
        let height = (room.1 - GAP * (rows - 1) as f32) / rows as f32 - CAPTION_H;
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let w = width.min(height * aspect);
        if w > best.tile.0 {
            best = Grid {
                cols,
                rows,
                tile: (w, w / aspect),
            };
        }
    }
    best
}

/// One picture on the wall.
enum Tile<'a> {
    Browser(&'a Task, std::sync::Arc<Preview>),
    App(&'a Task, std::sync::Arc<AppPreview>),
}

pub(crate) fn view(state: &Wall, cx: &mut Cx<Wall>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let root = col()
        .key("wall-root")
        .w_full()
        .h_full()
        .padding(PAD)
        .items_center()
        .justify_start();
    let Some(lead) = &state.lead else {
        return root;
    };
    let k = p.k;
    let c = &k.c;
    let team = team(&state.board, lead);
    let mut tiles: Vec<Tile> = Vec::new();
    for task in &team {
        tiles.extend(
            app_live::previews(&task.id)
                .into_iter()
                .map(|preview| Tile::App(task, preview)),
        );
        tiles.extend(
            live::previews(&task.id)
                .into_iter()
                .map(|preview| Tile::Browser(task, preview)),
        );
    }
    let title = team
        .first()
        .map(|task| task.title.clone())
        .unwrap_or_default();
    let close = state.actions.close;
    let header = row()
        .key("wall-header")
        .w_full()
        .h(HEADER_H)
        .gap(10.0)
        .center_y()
        .child(
            text(tr_fill("All {} live", &tiles.len().to_string()))
                .key("wall-count")
                .role("heading")
                .font(k.ty.ui_medium)
                .color(p.text()),
        )
        .child(
            text(title)
                .key("wall-title")
                .truncate()
                .font(k.ty.small)
                .color(p.muted())
                .min_w(0.0)
                .grow(1.0),
        )
        .child(
            IconButton::new(k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .label(tr("Close"))
                .on_press(move |s: &mut Wall| close(s))
                .build(k)
                .key("wall-close"),
        );
    let room = (SIZE.0 - PAD * 4.0, SIZE.1 - PAD * 4.0 - HEADER_H - GAP);
    let layout = grid(tiles.len(), room, ASPECT);
    let open = state.actions.open;
    let mut rows: Vec<El> = Vec::new();
    let mut tiles = tiles.into_iter().peekable();
    let mut at = 0;
    while tiles.peek().is_some() {
        let cells: Vec<El> = tiles
            .by_ref()
            .take(layout.cols)
            .map(|tile| cell(p, &tile, layout.tile, open))
            .collect();
        rows.push(row().key(format!("wall-row-{at}")).gap(GAP).children(cells));
        at += 1;
    }
    let grid = col()
        .key("wall-grid")
        .gap(GAP)
        .role("list")
        .label(tr("Live windows"))
        .children(rows);
    root.child(
        col()
            .key("wall-panel")
            .padding(PAD)
            .gap(GAP)
            .radius(16.0)
            .fill(c.bg_base)
            .border(p.hair, c.border_base)
            .box_shadow(rune::Shadow {
                x: 0.0,
                y: 6.0,
                blur: 24.0,
                spread: 0.0,
                color: rune::Color::BLACK.alpha(if k.dark { 0.45 } else { 0.18 }),
                inset: false,
            })
            // The panel takes the pointer, so × and the space between the
            // tiles answer; without it a click fell through to the app below.
            .hit()
            .on_press(|_: &mut Wall| {})
            .child(header)
            .child(grid)
            .on_key(move |s: &mut Wall, key: &rune::KeyEvent, _| {
                if key.key == rune::Key::Escape {
                    close(s);
                    true
                } else {
                    false
                }
            })
            .transition(rune::Transition::scale(0.97).combined(rune::Transition::opacity())),
    )
}

/// One tile: the picture (sharp from its full frame), or for a light
/// browser that draws no pixels its page's title and site; under it the
/// helper's dot and title and what it shows.
fn cell(p: Paint, tile: &Tile, size: (f32, f32), open: fn(&mut Wall, &str, &str)) -> El {
    let k = p.k;
    let c = &k.c;
    let (task, session, what, picture) = match tile {
        Tile::Browser(task, preview) => {
            let page = preview.page();
            let what = live::host_of(&page.url)
                .or_else(|| (!preview.name.is_empty()).then(|| preview.name.clone()))
                .unwrap_or_else(|| tr("Main browser").to_owned());
            let base = el()
                .size(size.0, size.1)
                .radius(8.0)
                .fill(c.layer_02)
                .border(p.hair, k.alpha(c.text_base, 0.14));
            let picture = if preview.has_frame() {
                base.pixels_fit(preview.large_pixels(), rune::SurfaceFit::Fill)
            } else {
                base.center().padding(10.0).child(
                    col()
                        .gap(4.0)
                        .items_center()
                        .child(
                            text(if page.title.is_empty() {
                                what.clone()
                            } else {
                                page.title.clone()
                            })
                            .truncate()
                            .font(k.ty.small_medium)
                            .color(p.muted()),
                        )
                        .children((!page.url.is_empty()).then(|| {
                            text(page.url.clone())
                                .truncate()
                                .font(k.ty.small)
                                .color(p.faint())
                        })),
                )
            };
            (*task, preview.session.clone(), what, picture)
        }
        Tile::App(task, preview) => {
            let base = el()
                .size(size.0, size.1)
                .radius(8.0)
                .fill(c.layer_02)
                .border(p.hair, k.alpha(c.text_base, 0.14));
            let picture = if preview.has_frame() {
                base.pixels_fit(preview.large_pixels(), rune::SurfaceFit::Fit)
            } else {
                base.center().child(
                    text(preview.window.app.clone())
                        .truncate()
                        .font(k.ty.small)
                        .color(p.faint()),
                )
            };
            (
                *task,
                preview.session(),
                preview.window.app.clone(),
                picture,
            )
        }
    };
    let colour = palette::task(task.colour, k.dark);
    let ended = matches!(task.phase, Phase::Done | Phase::Failed | Phase::Stopped);
    let caption = row()
        .w(size.0)
        .h(CAPTION_H)
        .gap(5.0)
        .center_y()
        .child(orb::dot(task, colour, 6.0))
        .child(
            text(format!("{} · {what}", task.title))
                .truncate()
                .font(k.ty.small)
                .color(if ended { p.faint() } else { p.muted() })
                .min_w(0.0),
        );
    let (id, session_id) = (task.id.clone(), session.clone());
    k.interactive(
        col()
            .key(format!("wall-tile-{session}"))
            .w(size.0)
            .child(picture)
            .child(caption)
            .role("button")
            .label(tr_fill("Open {} live", &format!("{} · {what}", task.title)))
            .hit()
            .on_press(move |s: &mut Wall| open(s, &id, &session_id)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::{Event, Origin};
    use std::time::Instant;

    #[test]
    fn twenty_pictures_fill_the_wall_as_large_as_they_fit() {
        let room = (1100.0, 680.0);
        let twenty = grid(20, room, ASPECT);
        assert_eq!((twenty.cols, twenty.rows), (5, 4));
        // Every tile fits, with its caption, gaps included.
        let used_w = twenty.tile.0 * 5.0 + GAP * 4.0;
        let used_h = (twenty.tile.1 + CAPTION_H) * 4.0 + GAP * 3.0;
        assert!(used_w <= room.0 + 0.01 && used_h <= room.1 + 0.01);
        assert!(twenty.tile.0 > 200.0, "readable: {:?}", twenty.tile);
        // One picture takes the room; four make a square.
        assert_eq!(grid(1, room, ASPECT).cols, 1);
        assert_eq!(
            (grid(4, room, ASPECT).cols, grid(4, room, ASPECT).rows),
            (2, 2)
        );
        // Thirty-two still fit.
        let many = grid(32, room, ASPECT);
        assert!(many.cols * many.rows >= 32);
        assert!(many.tile.0 > 100.0);
        // Nothing to show is one empty cell, not a division by zero.
        assert_eq!(grid(0, room, ASPECT).cols, 1);
    }

    #[test]
    fn the_wall_shows_a_lead_and_its_helpers_only() {
        let mut board = Board::default();
        let now = Instant::now();
        for id in ["lead", "h1", "h2", "other"] {
            board.apply(
                &Event::Started {
                    conversation: id.into(),
                    origin: Origin::Background,
                    question: format!("Job {id}"),
                },
                None,
                now,
            );
        }
        for task in &mut board.tasks {
            if task.id.starts_with('h') {
                task.lead = Some("lead".into());
            }
        }
        assert_eq!(tasks(&board, "lead"), ["lead", "h1", "h2"]);
        assert_eq!(tasks(&board, "other"), ["other"]);
        assert!(tasks(&board, "nobody").is_empty());
    }
}
