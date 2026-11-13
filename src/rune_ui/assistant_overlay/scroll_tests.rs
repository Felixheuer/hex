//! Nine and twenty browsers in every surface that shows them, scrolled
//! the way the app scrolls them: a wheel or trackpad delta routed by
//! `wheel::route` (as the host's scroll monitor does, since Rune's overlay
//! host drops scroll events) and applied through the panel's `Tx`. Every
//! picture comes into view: the job row's filmstrip (sideways, by the
//! trackpad and by the wheel), the live overview in the rail, the wall of
//! one job and of everything, and the task list's hovered row. Only the
//! pictures in view stream. A mini without a frame is an opaque card, and
//! a job with nothing to show has no strip. With a GPU (ignored) the
//! overview and the rows render to PNGs.

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};

use super::board::Board;
use super::fixture;
use super::minis::Desk;
use super::orb::{self, Orb};
use super::peek::Peek;
use super::wall::{self, Scope, Wall};
use super::wheel::{self, Surface};

fn runtime<S: 'static>(
    state: S,
    view: fn(&S, &mut Cx<S>) -> El,
    size: (f32, f32),
    dark: bool,
) -> Runtime<S> {
    let mut rt = crate::rune_ui::offscreen::runtime(
        state,
        view,
        size,
        Env {
            system_dark: dark,
            scale: 2.0,
        },
        TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
    );
    settle(&mut rt);
    rt
}

fn settle<S: 'static>(rt: &mut Runtime<S>) {
    rt.settle_all();
    rt.frame(0.0);
    // Layout reports (where each viewport is drawn) land a frame later.
    rt.frame(1.0 / 60.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn rect<S>(rt: &Runtime<S>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .map(|e| e.rect)
}

fn keys<S>(rt: &Runtime<S>, prefix: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for key in rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .filter(|k| k.starts_with(prefix))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn within([x, y, w, h]: [f32; 4], [vx, vy, vw, vh]: [f32; 4]) -> bool {
    x >= vx - 0.5 && y >= vy - 0.5 && x + w <= vx + vw + 0.5 && y + h <= vy + vh + 0.5
}

fn press<S: 'static>(rt: &mut Runtime<S>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    settle(rt);
}

/// Scrolls `viewport` in `surface` by `delta` per event, as the host's
/// monitor does, until `tile` is wholly in view; whether it got there.
fn reach<S: 'static>(
    rt: &mut Runtime<S>,
    surface: Surface,
    viewport: &str,
    tile: &str,
    delta: (f32, f32),
) -> bool {
    // Where the viewport is drawn reaches the registry once it rests.
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    for _ in 0..400 {
        let view = rect(rt, viewport).unwrap_or_else(|| panic!("no {viewport}"));
        let at = rect(rt, tile).unwrap_or_else(|| panic!("no {tile}"));
        if within(at, view) {
            return true;
        }
        let centre = (view[0] + view[2] / 2.0, view[1] + view[3] / 2.0);
        let Some(scroll) = wheel::route(surface, centre, delta) else {
            eprintln!(
                "stuck: {tile} {at:?} in {viewport} {view:?}: {:?}",
                wheel::dump(surface)
            );
            return false;
        };
        assert_eq!(
            scroll.key, viewport,
            "the wheel moves the viewport under it"
        );
        rt.update(move |_, tx| scroll.apply(tx));
        rt.frame(1.0 / 60.0);
    }
    false
}

fn orb_of(board: Board, desk: Desk) -> Orb {
    let mut actions = orb::Actions::<Orb>::inert();
    actions.list.toggle = |orb| orb.expanded = !orb.expanded;
    actions.jobs.open = |orb, scope, pressed| {
        // As the host does: the rows' pictures leave with them.
        orb.seen.clear();
        orb.peek = Some(Peek::new(scope.clone(), pressed));
    };
    actions.jobs.peek.close = |orb| orb.peek = None;
    actions.jobs.peek.seen = |orb, task, session, on| {
        let picture = (task.to_owned(), session.to_owned());
        if on {
            orb.seen.insert(picture);
        } else {
            orb.seen.remove(&picture);
        }
    };
    let mut orb = Orb::new(board, fixture::ANSWER_KEY.into(), actions);
    orb.shown = true;
    orb.desk = desk;
    orb
}

#[test]
fn a_rows_filmstrip_scrolls_sideways_to_every_browser() {
    for n in [9, 20] {
        let id = format!("film{n}");
        let board = fixture::browsers_board(&id, n, false);
        let mut rt = runtime(orb_of(board, Desk::default()), orb::view, orb::SIZE, false);
        let film = format!("job-film-{id}");
        let tiles = keys(&rt, "job-thumb-cell-");
        assert_eq!(tiles.len(), n, "every browser is on the row");
        let strip = rect(&rt, &film).unwrap();
        let card = rect(&rt, &format!("job-{id}")).unwrap();
        assert!(
            within(strip, card),
            "the strip stays inside its row: {strip:?}"
        );
        assert!(
            !within(rect(&rt, tiles.last().unwrap()).unwrap(), strip),
            "the last one starts out of view"
        );
        // Only the pictures in view stream.
        let shown = rt.state.seen.len();
        assert!((2..=3).contains(&shown), "{shown} of {n} in view");
        // A trackpad's sideways swipe reaches the last; a mouse wheel
        // (up and down) brings the first back.
        assert!(reach(
            &mut rt,
            Surface::Orb,
            &film,
            tiles.last().unwrap(),
            (-36.0, -2.0)
        ));
        assert!(reach(&mut rt, Surface::Orb, &film, &tiles[0], (0.0, 40.0)));
        for tile in &tiles {
            assert!(
                reach(&mut rt, Surface::Orb, &film, tile, (0.0, -40.0)),
                "{tile} of {n}"
            );
        }
    }
}

#[test]
fn all_live_on_a_row_opens_the_overview_which_shows_every_browser() {
    for n in [9, 20] {
        let id = format!("peek{n}");
        let board = fixture::browsers_board(&id, n, false);
        let mut rt = runtime(orb_of(board, Desk::default()), orb::view, orb::SIZE, true);
        press(&mut rt, "job-wall");
        assert_eq!(
            rt.state
                .peek
                .as_ref()
                .map(|peek| (&peek.scope, peek.pressed)),
            Some((&Scope::Job(id.clone()), true))
        );
        assert!(
            rect(&rt, "jobs").is_none(),
            "the overview takes the rows' place"
        );
        let tiles = keys(&rt, "peek-tile-");
        assert_eq!(tiles.len(), n);
        let grid = rect(&rt, "peek-grid").unwrap();
        let panel = rect(&rt, "peek").unwrap();
        assert!(
            within(panel, [0.0, 0.0, orb::SIZE.0, orb::SIZE.1]),
            "{panel:?}"
        );
        // Only what shows streams: the tiles in view, not all of them.
        let shown = rt.state.seen.len();
        assert!(
            shown >= 2 && (shown < n || n <= 10),
            "{shown} of {n} in view"
        );
        for tile in &tiles {
            assert!(
                reach(&mut rt, Surface::Orb, "peek-grid", tile, (0.0, -40.0)),
                "{tile} of {n}"
            );
        }
        settle(&mut rt);
        let last = (id.clone(), format!("hex-{id}_{}", n - 1));
        assert!(rt.state.seen.contains(&last), "scrolled to, it streams");
        assert!(
            !rt.state.seen.contains(&(id.clone(), format!("hex-{id}"))) || n <= 10,
            "scrolled away, the first rests: {:?}",
            rt.state.seen
        );
        assert!(grid[3] > 200.0, "{grid:?}");
        press(&mut rt, "peek-close");
        assert!(rt.state.peek.is_none());
        assert!(rect(&rt, "jobs").is_some(), "the rows come back");
    }
}

#[test]
fn every_tile_on_the_wall_is_reachable() {
    for n in [9, 20] {
        let id = format!("wall{n}");
        for scope in [Scope::Job(id.clone()), Scope::All] {
            let board = fixture::browsers_board(&id, n, false);
            let mut state = Wall::new(wall::Actions {
                close: |state| state.scope = None,
                open: |_, _, _| {},
                seen: |_, _, _, _| {},
            });
            state.scope = Some(scope.clone());
            state.board = board;
            // A shorter display than this Mac's: the wall fits it.
            state.size = wall::fitted((1440.0, 860.0), 40.0);
            let size = state.size;
            let mut rt = runtime(state, wall::view, size, false);
            let tiles = keys(&rt, "wall-tile-");
            assert_eq!(tiles.len(), n, "{scope:?}");
            let panel = rect(&rt, "wall-panel").unwrap();
            assert!(within(panel, [0.0, 0.0, size.0, size.1]), "{panel:?}");
            let tile = rect(&rt, &tiles[0]).unwrap();
            assert!(tile[2] >= wall::MIN_TILE_W - 0.5, "readable: {tile:?}");
            for tile in &tiles {
                assert!(
                    reach(&mut rt, Surface::Wall, "wall-grid", tile, (0.0, -60.0)),
                    "{tile} of {n}, {scope:?}"
                );
            }
        }
    }
}

#[test]
fn the_hovered_rows_browsers_in_the_list_are_reachable() {
    for n in [9, 20] {
        let id = format!("list{n}");
        let board = fixture::browsers_board(&id, n, false);
        let mut orb = orb_of(board, Desk::default());
        orb.expanded = true;
        orb.hovered = Some(id.clone());
        let mut rt = runtime(orb, orb::view, orb::SIZE, false);
        let tiles = keys(&rt, "preview-");
        assert_eq!(tiles.len(), n);
        for tile in &tiles {
            assert!(
                reach(&mut rt, Surface::Orb, "sheet-rows", tile, (0.0, -40.0)),
                "{tile} of {n}"
            );
        }
    }
}

#[test]
fn a_mini_without_a_frame_is_an_opaque_card_and_a_job_without_pictures_has_no_strip() {
    // An app window whose picture hasn't come: its name on an opaque card,
    // never a hole the desktop shows through.
    let board = fixture::browsers_board("waits", 1, true);
    let rt = runtime(orb_of(board, Desk::default()), orb::view, orb::SIZE, false);
    assert!(rect(&rt, "mini-none").is_some(), "a placeholder card");
    let texts: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    assert!(texts.iter().any(|t| t == "Ghostty"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.starts_with("Waiting for a picture")),
        "{texts:?}"
    );
    // A job that only searched and ran commands: no strip at all.
    let board = fixture::browsers_board("bare", 0, false);
    let rt = runtime(orb_of(board, Desk::default()), orb::view, orb::SIZE, false);
    assert!(rect(&rt, "job-bare").is_some());
    assert!(rect(&rt, "job-film-bare").is_none());
    assert!(keys(&rt, "job-thumb-").is_empty() && rect(&rt, "mini-none").is_none());
    // A see-through capture is drawn opaque.
    let mut rgba = vec![10, 20, 30, 0, 40, 50, 60, 128];
    super::app_live::opaque(&mut rgba);
    assert_eq!(rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
}

/// Renders the overview, the filmstrip, and the wall with 3, 9, and 20
/// browsers to `AGENT_DESK_ASSISTANT_SNAPS/peek-*.png`, light and dark.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn peek_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        for n in [3, 9, 20] {
            let id = format!("snap{n}");
            let board = fixture::browsers_board(&id, n, false);
            // The row with its filmstrip, scrolled a little.
            let mut rt = runtime(
                orb_of(board.clone(), Desk::default()),
                orb::view,
                orb::SIZE,
                dark,
            );
            let film = format!("job-film-{id}");
            if let Some(scroll) = wheel::route(Surface::Orb, centre(&rt, &film), (-90.0, 0.0)) {
                rt.update(move |_, tx| scroll.apply(tx));
            }
            save(
                &gpu,
                &mut rt,
                &folder,
                &format!("peek-row-{n}-{theme}"),
                orb::SIZE,
                dark,
            );
            // The overview.
            let mut orb = orb_of(board.clone(), Desk::default());
            orb.peek = Some(Peek::new(Scope::Job(id.clone()), true));
            let mut rt = runtime(orb, orb::view, orb::SIZE, dark);
            save(
                &gpu,
                &mut rt,
                &folder,
                &format!("peek-overview-{n}-{theme}"),
                orb::SIZE,
                dark,
            );
            // The wall of the job.
            let mut state = Wall::new(wall::Actions {
                close: |_| {},
                open: |_, _, _| {},
                seen: |_, _, _, _| {},
            });
            state.scope = Some(Scope::Job(id.clone()));
            state.board = board;
            let mut rt = runtime(state, wall::view, wall::SIZE, dark);
            save(
                &gpu,
                &mut rt,
                &folder,
                &format!("peek-wall-{n}-{theme}"),
                wall::SIZE,
                dark,
            );
        }
        // A mini waiting for its first frame, and a job with nothing to show.
        let mut board = fixture::browsers_board("snapwait", 1, true);
        board
            .tasks
            .extend(fixture::browsers_board("snapbare", 0, false).tasks.clone());
        let mut rt = runtime(orb_of(board, Desk::default()), orb::view, orb::SIZE, dark);
        save(
            &gpu,
            &mut rt,
            &folder,
            &format!("peek-waiting-{theme}"),
            orb::SIZE,
            dark,
        );
    }
}

fn centre<S>(rt: &Runtime<S>, key: &str) -> (f32, f32) {
    let r = rect(rt, key).unwrap_or_default();
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn save<S: 'static>(
    gpu: &rune::render::Gpu,
    rt: &mut Runtime<S>,
    folder: &std::path::Path,
    name: &str,
    size: (f32, f32),
    dark: bool,
) {
    settle(rt);
    let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
    let mut renderer = crate::rune_ui::offscreen::renderer(&gpu.device);
    let target = crate::rune_ui::offscreen::target(&gpu.device, w, h);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (w, h),
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let grey = if dark { 0.08 } else { 0.62 };
    renderer.draw(&mut encoder, &target.view, [grey, grey, grey, 1.0]);
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    let pixels = target.read(&gpu.device);
    rune::render::write_png(&folder.join(format!("{name}.png")), w, h, &pixels).unwrap();
}
