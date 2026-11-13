//! The orb crowded with live pictures, as on this Mac's 1800 × 1169 point
//! display: one to twelve running jobs with their browsers and app windows,
//! a lead with helpers, the desk's apps and browsers, a question, and a
//! result. Everything stays inside the panel as placed on the display, the
//! job rows scroll under the wheel, "All live" opens the wall of everything,
//! and the wall groups every picture per job and closes on ×, Escape, and a
//! click beside it. With a GPU (ignored) they render to PNGs to look at.

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};

use super::board::Board;
use super::fixture;
use super::minis::Desk;
use super::orb::{self, Orb};
use super::place;
use super::wall::{self, Scope, Wall};

/// The built-in display's visible area: below a 37 point menu bar.
const VISIBLE: [f64; 4] = [0.0, 37.0, 1800.0, 1132.0];

/// Opened "All live", for the tests to see.
static ALL_PRESSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn orb_at(board: Board, desk: Desk, room: f32, open: bool) -> Orb {
    let mut actions = orb::Actions::<Orb>::inert();
    actions.list.toggle = |orb| orb.expanded = !orb.expanded;
    actions.jobs.all = |_| {
        ALL_PRESSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    };
    let mut orb = Orb::new(board, fixture::ANSWER_KEY.into(), actions);
    orb.shown = true;
    orb.expanded = open;
    orb.room = room;
    orb.desk = desk;
    orb
}

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
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn rect<S>(rt: &Runtime<S>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .map(|e| e.rect)
}

fn keys<S>(rt: &Runtime<S>, prefix: &str) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key.clone())
        .filter(|k| k.starts_with(prefix))
        .collect()
}

fn inside(r: [f32; 4], size: (f32, f32)) -> bool {
    let [x, y, w, h] = r;
    x >= -0.5 && y >= -0.5 && x + w <= size.0 + 0.5 && y + h <= size.1 + 0.5
}

fn press<S: 'static>(rt: &mut Runtime<S>, at: (f32, f32)) {
    rt.pointer_move(at.0, at.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(at.0, at.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(at.0, at.1);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    rt.settle_all();
    rt.frame(0.0);
}

/// The panel's heights on the 1800 × 1169 display: the orb at the top
/// beside the HUD, and dragged to just above the middle.
fn rooms() -> [f32; 2] {
    let top = place::default_anchor(VISIBLE, Some((900.0, true)));
    let middle = (400.0, 560.0);
    [top, middle].map(|anchor| place::fitted(anchor, place::grow(anchor, VISIBLE), VISIBLE)[3])
}

#[test]
fn a_crowded_orb_stays_inside_its_panel_and_its_rows_scroll() {
    let [tall, short] = rooms();
    assert_eq!(tall, orb::SIZE.1, "the tallest panel fits at the top");
    assert!(short < tall && short > 500.0, "{short}");
    for jobs in [1, 3, 6, 12] {
        for room in [tall, short] {
            for question in [false, true] {
                let mut board = fixture::crowd_board(jobs, if jobs >= 3 { 5 } else { 0 });
                if question && jobs >= 2 {
                    fixture::crowd_question(&mut board);
                }
                let size = (orb::SIZE.0, room);
                let mut rt = runtime(
                    orb_at(board, fixture::crowd_desk(), room, false),
                    orb::view,
                    size,
                    true,
                );
                let what = format!("{jobs} jobs in {room}, question {question}");
                for key in ["orb", "orb-all-live", "jobs", "cluster", "desk-minis"] {
                    let r = rect(&rt, key).unwrap_or_else(|| panic!("{key}: {what}"));
                    assert!(inside(r, size), "{key} {r:?}: {what}");
                }
                // The rows with their pictures, apps first, and every job
                // past them in the line.
                assert!(!keys(&rt, "job-crowd").is_empty(), "{what}");
                assert!(!keys(&rt, "job-thumb-cell-").is_empty(), "{what}");
                if jobs > 3 {
                    assert!(rect(&rt, "jobs-line").is_some(), "{what}");
                }
                // The column takes the pointer, so the wheel scrolls it.
                let jobs_rect = rect(&rt, "jobs").unwrap();
                let centre = (jobs_rect[0] + 20.0, jobs_rect[1] + jobs_rect[3] / 2.0);
                assert!(
                    rt.hit_regions().iter().any(|[x0, y0, x1, y1]| {
                        centre.0 >= *x0 && centre.0 <= *x1 && centre.1 >= *y0 && centre.1 <= *y1
                    }),
                    "the rows take the pointer: {what}"
                );
                let desk_before = rect(&rt, "desk-minis").unwrap();
                rt.wheel(centre.0, centre.1, -400.0);
                rt.settle_all();
                rt.frame(0.0);
                let desk_after = rect(&rt, "desk-minis").unwrap();
                let overflows = desk_before[1] + desk_before[3] > jobs_rect[1] + jobs_rect[3] + 0.5;
                if overflows {
                    assert!(desk_after[1] < desk_before[1], "scrolled: {what}");
                } else {
                    assert_eq!(desk_after, desk_before, "nothing to scroll: {what}");
                }
            }
        }
    }
}

#[test]
fn all_live_beside_the_orb_counts_every_picture_and_opens_the_wall() {
    let board = fixture::crowd_board(4, 3);
    let desk = fixture::crowd_desk();
    // Four jobs (3 + 1 + 2 + 3 browsers, one Numbers window), three helpers
    // with a browser each, and the desk's WhatsApp and browser.
    assert_eq!(wall::count_all(&board, &desk), 9 + 1 + 3 + 2);
    let mut rt = runtime(
        orb_at(board, desk, orb::SIZE.1, false),
        orb::view,
        orb::SIZE,
        false,
    );
    let before = ALL_PRESSED.load(std::sync::atomic::Ordering::Relaxed);
    let (x, y) = rt.find("orb-all-live").expect("All live beside the orb");
    press(&mut rt, (x, y));
    assert!(ALL_PRESSED.load(std::sync::atomic::Ordering::Relaxed) > before);
    // Nothing live: no chip.
    let rt = runtime(
        orb_at(fixture::quiet_board(), Desk::default(), orb::SIZE.1, false),
        orb::view,
        orb::SIZE,
        false,
    );
    assert!(rect(&rt, "orb-all-live").is_none());
}

#[test]
fn the_full_list_scrolls_inside_the_shorter_panel() {
    let [_, short] = rooms();
    let size = (orb::SIZE.0, short);
    let mut rt = runtime(
        orb_at(
            fixture::crowd_board(12, 0),
            fixture::crowd_desk(),
            short,
            true,
        ),
        orb::view,
        size,
        false,
    );
    let sheet = rect(&rt, "sheet").expect("the list");
    assert!(inside(sheet, size), "{sheet:?}");
    let rows = rect(&rt, "sheet-rows").unwrap();
    let centre = (rows[0] + rows[2] / 2.0, rows[1] + rows[3] / 2.0);
    assert!(rt.hit_regions().iter().any(|[x0, y0, x1, y1]| {
        centre.0 >= *x0 && centre.0 <= *x1 && centre.1 >= *y0 && centre.1 <= *y1
    }));
    let first = keys(&rt, "task-crowd")[0].clone();
    let before = rect(&rt, &first).unwrap();
    rt.wheel(centre.0, centre.1, -300.0);
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, &first).unwrap()[1] < before[1], "it scrolls");
}

static CLOSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static OPENED: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

fn wall_of(scope: Scope, board: Board, desk: Desk) -> Wall {
    let mut state = Wall::new(wall::Actions {
        close: |state| {
            state.scope = None;
            CLOSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        },
        open: |_, task, session| {
            OPENED
                .lock()
                .unwrap()
                .push((task.to_owned(), session.to_owned()));
        },
    });
    state.scope = Some(scope);
    state.board = board;
    state.desk = desk;
    state
}

#[test]
fn the_wall_of_everything_groups_each_job_and_the_desk_and_closes() {
    let board = fixture::crowd_board(12, 5);
    let desk = fixture::crowd_desk();
    let all = wall::count_all(&board, &desk);
    let mut rt = runtime(
        wall_of(Scope::All, board, desk),
        wall::view,
        wall::SIZE,
        true,
    );
    // A header per job with pictures, and the desk's.
    assert_eq!(keys(&rt, "wall-block-").len(), 12 + 1);
    let tiles = keys(&rt, "wall-tile-");
    assert_eq!(tiles.len(), all);
    let panel = rect(&rt, "wall-panel").unwrap();
    assert!(inside(panel, wall::SIZE), "{panel:?}");
    // Readable pictures: they scroll rather than shrink.
    let tile = rect(&rt, &tiles[0]).unwrap();
    assert!(tile[2] >= wall::MIN_TILE_W - 0.5, "{tile:?}");
    let grid = rect(&rt, "wall-grid").unwrap();
    let last = tiles.last().unwrap().clone();
    let hidden = rect(&rt, &last).unwrap();
    assert!(hidden[1] > grid[1] + grid[3], "the last is past the fold");
    let centre = (grid[0] + grid[2] / 2.0, grid[1] + grid[3] / 2.0);
    rt.wheel(centre.0, centre.1, -2000.0);
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, &last).unwrap()[1] < hidden[1], "it scrolls");
    // A tile opens its picture large.
    let tile = keys(&rt, "wall-tile-hex-crowd-helper")[0].clone();
    let r = rect(&rt, &tile).unwrap();
    let before = CLOSED.load(std::sync::atomic::Ordering::Relaxed);
    if r[1] > grid[1] && r[1] + 40.0 < grid[1] + grid[3] {
        press(&mut rt, (r[0] + 20.0, r[1] + 20.0));
        assert!(
            OPENED
                .lock()
                .unwrap()
                .iter()
                .any(|(task, _)| task.starts_with("crowd-helper"))
        );
    }
    // Between the tiles nothing closes; beside the panel it does.
    press(&mut rt, (panel[0] + 6.0, panel[1] + 6.0));
    assert!(rt.state.scope.is_some(), "a press inside stays");
    assert_eq!(CLOSED.load(std::sync::atomic::Ordering::Relaxed), before);
    press(&mut rt, (panel[0] + 6.0, wall::SIZE.1 - 4.0));
    assert!(rt.state.scope.is_none(), "a press beside it closes");
}

#[test]
fn the_wall_closes_on_its_x() {
    let mut rt = runtime(
        wall_of(
            Scope::Job("crowd0".into()),
            fixture::crowd_board(2, 3),
            Desk::default(),
        ),
        wall::view,
        wall::SIZE,
        false,
    );
    assert_eq!(keys(&rt, "wall-tile-").len(), 3 + 3);
    let at = rt.find("wall-close").unwrap();
    press(&mut rt, at);
    assert!(rt.state.scope.is_none());
}

/// Renders the crowded orb and the wall to `AGENT_DESK_ASSISTANT_SNAPS/crowd-*.png`.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn crowd_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    let [tall, short] = rooms();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let save = |name: &str, size: (f32, f32), rt: &mut dyn FnMut(u32, u32) -> Vec<u8>| {
            let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
            let pixels = rt(w, h);
            let path = folder.join(format!("crowd-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        };
        let scenes: Vec<(String, Board, Desk, f32, bool)> = vec![
            (
                "1-job".into(),
                fixture::crowd_board(1, 0),
                Desk::default(),
                tall,
                false,
            ),
            (
                "3-jobs-desk".into(),
                fixture::crowd_board(3, 0),
                fixture::crowd_desk(),
                tall,
                false,
            ),
            (
                "6-jobs-helpers-desk".into(),
                fixture::crowd_board(6, 5),
                fixture::crowd_desk(),
                tall,
                false,
            ),
            (
                "12-jobs-question-desk".into(),
                {
                    let mut board = fixture::crowd_board(12, 5);
                    fixture::crowd_question(&mut board);
                    board
                },
                fixture::crowd_desk(),
                tall,
                false,
            ),
            (
                "12-jobs-short-panel".into(),
                fixture::crowd_board(12, 5),
                fixture::crowd_desk(),
                short,
                false,
            ),
            (
                "12-jobs-list".into(),
                fixture::crowd_board(12, 5),
                fixture::crowd_desk(),
                short,
                true,
            ),
        ];
        for (name, board, desk, room, open) in scenes {
            let size = (orb::SIZE.0, room);
            let mut rt = runtime(orb_at(board, desk, room, open), orb::view, size, dark);
            save(&name, size, &mut |w, h| backdrop(&gpu, &mut rt, w, h, dark));
        }
        for (name, scope, board, desk) in [
            (
                "wall-all",
                Scope::All,
                fixture::crowd_board(12, 5),
                fixture::crowd_desk(),
            ),
            (
                "wall-all-few",
                Scope::All,
                fixture::crowd_board(2, 0),
                fixture::crowd_desk(),
            ),
            (
                "wall-job",
                Scope::Job("crowd0".into()),
                fixture::crowd_board(3, 5),
                Desk::default(),
            ),
        ] {
            let mut rt = runtime(wall_of(scope, board, desk), wall::view, wall::SIZE, dark);
            save(name, wall::SIZE, &mut |w, h| {
                backdrop(&gpu, &mut rt, w, h, dark)
            });
        }
    }
}

/// `rt` drawn over a desktop grey, as the clear overlay sits on screen.
fn backdrop<S: 'static>(
    gpu: &rune::render::Gpu,
    rt: &mut Runtime<S>,
    w: u32,
    h: u32,
    dark: bool,
) -> Vec<u8> {
    rt.settle_all();
    rt.frame(0.0);
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
    target.read(&gpu.device)
}
