//! The live wall: browsers and app windows live at once, as a grid of
//! small pictures, each captioned with its task and site; a press on one
//! opens it large (the enlarged card, with Take over).
//!
//! It shows one of two things ([`Scope`]):
//!
//! - A job and its helpers ("All 20 live" on a job's row): twenty helpers
//!   each with a browser are twenty pictures sized to fill the panel.
//! - Everything live ("All live" beside the orb): every running job with
//!   its helpers, then the person's desk (the apps and browsers quick turns
//!   left open), each under a header with its colour, title, and count.
//!   Pictures keep a readable size and the grid scrolls when they don't
//!   all fit.
//!
//! ×, Escape, or a click outside the panel closes it. The pictures stream
//! only while the wall shows: opening it puts its tasks on `live::wall` and
//! `app_live::wall` (two frames a second, at full size so a tile stays
//! sharp); closing it takes them off again.
//!
//! The rules are plain functions ([`tasks`], [`groups`], [`grid`]); the
//! host adds the panel and the streams (`host::wall`).

use rune::{Cx, El, Icon, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::app_live::{self, AppPreview};
use super::board::{Board, Phase, Task};
use super::live::{self, Preview};
use super::minis::{self, Desk};
use super::{orb, palette};
use crate::desktop::i18n::{tr, tr_fill};

/// The panel.
pub(crate) const SIZE: (f32, f32) = (1180.0, 780.0);
/// Padding around the grid, and the header above it.
const PAD: f32 = 20.0;
const HEADER_H: f32 = 40.0;
/// A group's header on the wall of everything.
const GROUP_H: f32 = 26.0;
/// Space between tiles, and a tile's caption under its picture.
pub(crate) const GAP: f32 = 10.0;
pub(crate) const CAPTION_H: f32 = 18.0;
/// A browser page's shape (16:10), for tiles before their first frame.
const ASPECT: f32 = 1.6;
/// The narrowest a tile gets on the wall of everything: past that it
/// scrolls instead of shrinking.
pub(crate) const MIN_TILE_W: f32 = 210.0;
/// The grid's room inside the panel.
pub(crate) const ROOM: (f32, f32) = (SIZE.0 - PAD * 4.0, SIZE.1 - PAD * 4.0 - HEADER_H - GAP);

/// Four panes: "All live" at the orb.
pub(crate) static GRID: Icon = Icon {
    name: "hex-wall-grid",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor"/><rect x="9" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor"/>"#,
};

/// What the wall shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Scope {
    /// A job and its helpers (the job's id).
    Job(String),
    /// Every live task's pictures and the desk's.
    All,
}

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
    /// What it shows; `None` while closed.
    pub(crate) scope: Option<Scope>,
    /// The orb's board as it last said, for the teams and their titles.
    pub(crate) board: Board,
    /// The person's desk as last read.
    pub(crate) desk: Desk,
    pub(crate) actions: Actions<Wall>,
}

impl Wall {
    pub(crate) fn new(actions: Actions<Wall>) -> Self {
        Self {
            scope: None,
            board: Board::default(),
            desk: Desk::default(),
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

/// The live tasks no other task on `board` leads, newest first: a group
/// each on the wall of everything.
pub(crate) fn roots(board: &Board) -> Vec<&Task> {
    let led = |task: &Task| {
        task.lead
            .as_ref()
            .is_some_and(|lead| board.tasks.iter().any(|other| &other.id == lead))
    };
    board
        .tasks
        .iter()
        .rev()
        .filter(|task| task.phase.live() && !led(task))
        .collect()
}

/// The task ids `scope` streams: a job's team; or every root's team and
/// the desk's windows and browsers.
pub(crate) fn tasks(board: &Board, desk: &Desk, scope: &Scope) -> Vec<String> {
    match scope {
        Scope::Job(lead) => team(board, lead)
            .into_iter()
            .map(|task| task.id.clone())
            .collect(),
        Scope::All => {
            let mut ids: Vec<String> = roots(board)
                .into_iter()
                .flat_map(|root| team(board, &root.id))
                .map(|task| task.id.clone())
                .collect();
            if !desk.is_empty() {
                ids.push(minis::DESK.to_owned());
                ids.extend(desk.browsers.iter().cloned());
            }
            ids
        }
    }
}

/// How many live pictures there are of every live task and the desk: what
/// "All live" at the orb opens.
pub(crate) fn count_all(board: &Board, desk: &Desk) -> usize {
    groups(board, desk, &Scope::All)
        .iter()
        .map(|group| group.tiles.len())
        .sum()
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

/// The grid for the wall of everything: `n` pictures in `groups` groups
/// (a header each) as large as they fit in `room`, but never narrower than
/// [`MIN_TILE_W`]: then as many columns of that width as fit, and the grid
/// scrolls.
pub(crate) fn scrolling_grid(n: usize, groups: usize, room: (f32, f32)) -> Grid {
    // Each group may start a row of its own: room for the headers and a
    // short row each.
    let rows_room = room.1 - groups.saturating_sub(1) as f32 * (GROUP_H + GAP) - GROUP_H;
    let fit = grid(n + groups.saturating_sub(1), (room.0, rows_room), ASPECT);
    if fit.tile.0 >= MIN_TILE_W {
        return fit;
    }
    let cols = (((room.0 + GAP) / (MIN_TILE_W + GAP)).floor() as usize).max(1);
    let w = (room.0 - GAP * (cols - 1) as f32) / cols as f32;
    Grid {
        cols,
        rows: n.div_ceil(cols),
        tile: (w, w / ASPECT),
    }
}

/// One picture on the wall.
pub(crate) enum Picture {
    Browser(std::sync::Arc<Preview>),
    App(std::sync::Arc<AppPreview>),
}

/// One picture and whose it is: a task on the board, or the desk's.
pub(crate) struct Tile<'a> {
    /// The task its stream belongs to (the desk's own id for its windows).
    pub(crate) owner: String,
    /// The task on the board, for its colour and title; `None` on the desk.
    pub(crate) task: Option<&'a Task>,
    pub(crate) picture: Picture,
}

/// A run of tiles under one header.
pub(crate) struct Group<'a> {
    /// The job it's of; `None` for the desk.
    pub(crate) task: Option<&'a Task>,
    pub(crate) tiles: Vec<Tile<'a>>,
}

fn team_tiles<'a>(board: &'a Board, lead: &str) -> Vec<Tile<'a>> {
    let mut tiles = Vec::new();
    for task in team(board, lead) {
        tiles.extend(
            app_live::previews(&task.id)
                .into_iter()
                .map(|preview| Tile {
                    owner: task.id.clone(),
                    task: Some(task),
                    picture: Picture::App(preview),
                }),
        );
        tiles.extend(live::previews(&task.id).into_iter().map(|preview| Tile {
            owner: task.id.clone(),
            task: Some(task),
            picture: Picture::Browser(preview),
        }));
    }
    tiles
}

fn desk_tiles(desk: &Desk) -> Vec<Tile<'static>> {
    if desk.is_empty() {
        return Vec::new();
    }
    let mut tiles: Vec<Tile> = app_live::previews(minis::DESK)
        .into_iter()
        .map(|preview| Tile {
            owner: minis::DESK.to_owned(),
            task: None,
            picture: Picture::App(preview),
        })
        .collect();
    for task in &desk.browsers {
        tiles.extend(live::previews(task).into_iter().map(|preview| Tile {
            owner: task.clone(),
            task: None,
            picture: Picture::Browser(preview),
        }));
    }
    tiles
}

/// What `scope` shows, group by group; groups without a picture are left
/// out.
pub(crate) fn groups<'a>(board: &'a Board, desk: &Desk, scope: &Scope) -> Vec<Group<'a>> {
    let mut groups: Vec<Group> = match scope {
        Scope::Job(lead) => vec![Group {
            task: board.tasks.iter().find(|task| &task.id == lead),
            tiles: team_tiles(board, lead),
        }],
        Scope::All => roots(board)
            .into_iter()
            .map(|root| Group {
                task: Some(root),
                tiles: team_tiles(board, &root.id),
            })
            .collect(),
    };
    if *scope == Scope::All {
        groups.push(Group {
            task: None,
            tiles: desk_tiles(desk),
        });
    }
    groups.retain(|group| !group.tiles.is_empty());
    groups
}

pub(crate) fn view(state: &Wall, cx: &mut Cx<Wall>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let close = state.actions.close;
    let root = col()
        .key("wall-root")
        .w_full()
        .h_full()
        .padding(PAD)
        .items_center()
        .justify_start();
    let Some(scope) = &state.scope else {
        return root;
    };
    // A click beside the panel closes it, as a click outside the window
    // does (it takes the keyboard away).
    let root = root.hit().on_press(move |s: &mut Wall| close(s));
    let k = p.k;
    let c = &k.c;
    let groups = groups(&state.board, &state.desk, scope);
    let count: usize = groups.iter().map(|group| group.tiles.len()).sum();
    let (heading, title) = match scope {
        Scope::Job(lead) => (
            tr_fill("All {} live", &count.to_string()),
            state
                .board
                .tasks
                .iter()
                .find(|task| &task.id == lead)
                .map(|task| task.title.clone())
                .unwrap_or_default(),
        ),
        Scope::All => (
            tr_fill("All {} live", &count.to_string()),
            tr_fill("{} jobs running", &roots(&state.board).len().to_string()),
        ),
    };
    let header = row()
        .key("wall-header")
        .w_full()
        .h(HEADER_H)
        .gap(10.0)
        .center_y()
        .child(
            text(heading)
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
    let open = state.actions.open;
    let headed = *scope == Scope::All;
    let layout = if headed {
        scrolling_grid(count, groups.len(), ROOM)
    } else {
        grid(count, ROOM, ASPECT)
    };
    let rows_of = |at: usize, group: &Group| -> Vec<El> {
        group
            .tiles
            .chunks(layout.cols)
            .enumerate()
            .map(|(line, tiles)| {
                row()
                    .key(format!("wall-row-{at}-{line}"))
                    .gap(GAP)
                    .children(tiles.iter().map(|tile| {
                        cell(
                            p,
                            tile,
                            layout.tile,
                            headed.then_some(group.task).flatten(),
                            open,
                        )
                    }))
            })
            .collect()
    };
    let mut sections: Vec<El> = Vec::new();
    if headed {
        // Each group a block under its header, as wide as its pictures;
        // blocks share a line while they fit, so a job with one browser
        // doesn't leave the rest of its line empty.
        let mut line: Vec<El> = Vec::new();
        let mut used = 0;
        for (at, group) in groups.iter().enumerate() {
            let slots = group.tiles.len().min(layout.cols);
            if used > 0 && used + slots > layout.cols {
                sections.push(
                    row()
                        .key(format!("wall-line-{}", sections.len()))
                        .gap(GAP)
                        .items_start()
                        .children(std::mem::take(&mut line)),
                );
                used = 0;
            }
            used += slots;
            let width = slots as f32 * layout.tile.0 + (slots - 1) as f32 * GAP;
            line.push(
                col()
                    .key(format!("wall-block-{at}"))
                    .w(width)
                    .gap(GAP / 2.0)
                    .child(group_header(p, group, at))
                    .children(rows_of(at, group)),
            );
        }
        if !line.is_empty() {
            sections.push(
                row()
                    .key(format!("wall-line-{}", sections.len()))
                    .gap(GAP)
                    .items_start()
                    .children(line),
            );
        }
    } else {
        sections.extend(groups.iter().enumerate().flat_map(|(at, g)| rows_of(at, g)));
    }
    let grid = col()
        .key("wall-grid")
        .w(ROOM.0)
        .min_h(0.0)
        .shrink()
        .gap(GAP)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(16.0)
        // The wheel answers between the tiles too.
        .hit()
        .role("list")
        .label(tr("Live windows"))
        .children(sections);
    root.child(
        col()
            .key("wall-panel")
            .max_h(SIZE.1 - PAD * 2.0)
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
            // tiles answer (and don't close it); without it a click fell
            // through to the app below.
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

/// A group's header on the wall of everything: the job's dot, title, and
/// how many pictures; or "Your desk".
fn group_header(p: Paint, group: &Group, at: usize) -> El {
    let k = p.k;
    let (dot, title) = match group.task {
        Some(task) => (
            Some(orb::marker(
                p,
                task,
                palette::task(task.colour, k.dark),
                8.0,
            )),
            task.title.clone(),
        ),
        None => (None, tr("Your desk").to_owned()),
    };
    row()
        .key(format!("wall-group-{at}"))
        .w_full()
        .h(GROUP_H)
        .gap(7.0)
        .center_y()
        .role("heading")
        .children(dot)
        .child(
            text(title)
                .key("wall-group-title")
                .truncate()
                .font(k.ty.small_medium)
                .color(p.text())
                .min_w(0.0),
        )
        .child(
            text(tr_fill("{} live", &group.tiles.len().to_string()))
                .key("wall-group-count")
                .font(k.ty.small)
                .color(p.faint()),
        )
}

/// One tile: the picture (sharp from its full frame), or for a light
/// browser that draws no pixels its page's title and site; under it the
/// task's dot and title and what it shows.
fn cell(
    p: Paint,
    tile: &Tile,
    size: (f32, f32),
    under: Option<&Task>,
    open: fn(&mut Wall, &str, &str),
) -> El {
    let k = p.k;
    let c = &k.c;
    let base = el()
        .size(size.0, size.1)
        .radius(8.0)
        .fill(c.layer_02)
        .border(p.hair, k.alpha(c.text_base, 0.14));
    let (session, what, picture) = match &tile.picture {
        Picture::Browser(preview) => {
            let page = preview.page();
            let what = live::host_of(&page.url)
                .or_else(|| (!preview.name.is_empty()).then(|| preview.name.clone()))
                .unwrap_or_else(|| tr("Main browser").to_owned());
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
            (preview.session.clone(), what, picture)
        }
        Picture::App(preview) => {
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
            (preview.session(), preview.window.app.clone(), picture)
        }
    };
    let (dot, words, ended) = match tile.task {
        Some(task) => (
            Some(orb::dot(task, palette::task(task.colour, k.dark), 6.0)),
            // Under its own job's header the site says enough.
            if under.is_some_and(|header| header.id == task.id) {
                what
            } else {
                format!("{} · {what}", task.title)
            },
            matches!(task.phase, Phase::Done | Phase::Failed | Phase::Stopped),
        ),
        None => (None, what, false),
    };
    let caption = row()
        .w(size.0)
        .h(CAPTION_H)
        .gap(5.0)
        .center_y()
        .children(dot)
        .child(
            text(words.clone())
                .truncate()
                .font(k.ty.small)
                .color(if ended { p.faint() } else { p.muted() })
                .min_w(0.0),
        );
    let (id, session_id) = (tile.owner.clone(), session.clone());
    k.interactive(
        col()
            .key(format!("wall-tile-{session}"))
            .w(size.0)
            .child(picture)
            .child(caption)
            .role("button")
            .label(tr_fill("Open {} live", &words))
            .hit()
            .on_press(move |s: &mut Wall| open(s, &id, &session_id)),
    )
}

/// "All N live" beside the orb: every live picture of every job and the
/// desk, a press away; `None` while there is none.
pub(crate) fn all_live_chip<S: 'static>(p: Paint, count: usize, open: fn(&mut S)) -> Option<El> {
    if count == 0 {
        return None;
    }
    let c = &p.k.c;
    let label = tr_fill("All {} live", &count.to_string());
    Some(
        p.k.interactive(
            row()
                .key("orb-all-live")
                .h(orb::ORB_H)
                .px(11.0)
                .gap(6.0)
                .center_y()
                .radius(orb::ORB_H / 2.0)
                .fill(c.bg_base.alpha(0.94))
                .border(p.hair, c.border_base)
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: 3.0,
                    blur: 12.0,
                    spread: 0.0,
                    color: rune::Color::BLACK.alpha(if p.k.dark { 0.4 } else { 0.12 }),
                    inset: false,
                })
                .role("button")
                .label(label.clone())
                .hit()
                .on_press(move |s: &mut S| open(s))
                .child(icon(&GRID, 14.0).color(p.text()).a11y_hidden())
                .child(
                    text(label)
                        .key("orb-all-live-text")
                        .font(p.k.ty.small_medium)
                        .color(p.text()),
                ),
        ),
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
    fn the_wall_of_everything_keeps_pictures_readable_and_scrolls() {
        // A few pictures in two groups: as large as they fit.
        let few = scrolling_grid(4, 2, ROOM);
        assert!(few.tile.0 >= MIN_TILE_W, "{few:?}");
        // Sixty pictures in twelve groups: never smaller than readable, as
        // many columns as fit; the rest scrolls.
        let many = scrolling_grid(60, 12, ROOM);
        assert!(many.tile.0 >= MIN_TILE_W, "{many:?}");
        assert!(many.cols >= 4, "{many:?}");
        assert!(many.tile.0 * many.cols as f32 + GAP * (many.cols - 1) as f32 <= ROOM.0 + 0.01);
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
        let desk = Desk::default();
        let job = |lead: &str| tasks(&board, &desk, &Scope::Job(lead.into()));
        assert_eq!(job("lead"), ["lead", "h1", "h2"]);
        assert_eq!(job("other"), ["other"]);
        assert!(job("nobody").is_empty());
        // Everything: each job with its helpers, newest job first, and the
        // desk's windows and browsers.
        assert_eq!(
            tasks(&board, &desk, &Scope::All),
            ["other", "lead", "h1", "h2"]
        );
        let desk = Desk {
            windows: Vec::new(),
            browsers: vec!["turn".into()],
        };
        assert_eq!(
            tasks(&board, &desk, &Scope::All),
            ["other", "lead", "h1", "h2", minis::DESK, "turn"]
        );
    }
}
