//! The three frames (`docs/ux/redesign.md` §3.5, agent C): the desk frame
//! with its tab only while the agent has the controls, the window outline
//! whose chip names its task in every look, and the rings on parked
//! windows' minis; laid out offscreen, and (ignored, with a GPU) rendered
//! to PNGs to look at, in light and dark.

use std::time::{Duration, Instant};

use rune::text::TextSystem;
use rune::{Color, Cx, El, Env, Runtime, col, el, row, text};

use super::super::controls::Paint;
use super::board::{Board, Look, Phase};
use super::frame::{self, Frame, Tab};
use super::outline::{self, LabelSide, Outline};
use super::pictures::{self, Ring};
use super::{jobs, look};
use crate::assistant::view::BACK_FOR;
use crate::assistant::{Event, Origin, TaskWindow};

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

fn text_of<S>(rt: &Runtime<S>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .and_then(|e| e.text)
}

const NUMBERS: &str = "w-7";

fn numbers() -> TaskWindow {
    TaskWindow {
        app: "Numbers".into(),
        window_id: Some(NUMBERS.into()),
    }
}

/// A job exporting a sheet, its Numbers window brought in front.
fn borrowing(now: Instant, until_ms: u64) -> Board {
    let mut board = Board::default();
    board.apply(
        &Event::Started {
            conversation: "export".into(),
            origin: Origin::Background,
            question: "Export the budget sheet as a PDF".into(),
        },
        None,
        now,
    );
    board.apply(
        &Event::Window {
            conversation: "export".into(),
            window: numbers(),
        },
        None,
        now,
    );
    board.apply(
        &Event::Step {
            conversation: "export".into(),
            description: "exporting PDF".into(),
        },
        None,
        now,
    );
    board.apply(
        &Event::Borrowed {
            conversation: "export".into(),
            window: numbers(),
            why: crate::assistant::guard::BORROW_IGNORED.into(),
            until_ms,
        },
        None,
        now,
    );
    board
}

fn display(tab: Option<Tab>, glow: bool, left: Option<u64>) -> Frame {
    Frame {
        display: Some(1),
        size: SCENE,
        seams: Default::default(),
        glow,
        tab,
        left,
    }
}

#[test]
fn a_borrow_shows_the_frame_and_its_tab_until_returned() {
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    let mut board = borrowing(now, 20_000);
    assert!(board.frame_until(now).is_some());
    let tab = Tab::of(&board, now).expect("a tab while in front");
    assert!(matches!(&tab, Tab::InFront { app, .. } if app == "Numbers"));
    assert_eq!(tab.left(17_100), Some(3), "rounded up");
    let rt = runtime(
        display(Some(tab.clone()), true, tab.left(18_000)),
        frame::view,
        SCENE,
        false,
    );
    assert!(rect(&rt, "frame").is_some(), "the glow");
    let [x, y, w, _] = rect(&rt, "frame-tab").expect("the tab");
    assert!(
        ((x + w / 2.0) - SCENE.0 / 2.0).abs() < 1.0,
        "centred under the island"
    );
    assert!(y >= frame::TAB_TOP - 0.5);
    // "Agent-Desk is using Numbers for a moment" is the island's line
    // (`island::line_with`); the tab defers to it and says the rest.
    assert_eq!(tab.line(), "Agent-Desk is using Numbers for a moment");
    assert!(text_of(&rt, "frame-tab-detail").is_none(), "one line");
    let detail = text_of(&rt, "frame-tab-line").unwrap();
    assert!(
        detail.contains("didn't respond in the background"),
        "{detail}"
    );
    assert!(detail.contains("Esc to stop"), "{detail}");
    assert!(detail.contains("back in 2 s"), "{detail}");
    // The front goes back: no frame, "Back to you · Mail" for a moment.
    board.apply(
        &Event::Returned {
            conversation: "export".into(),
            to: Some("Mail".into()),
        },
        None,
        now,
    );
    assert_eq!(board.frame_until(now), None);
    let back = Tab::of(&board, now).expect("back to you");
    assert!(!back.in_front());
    assert_eq!(back.line(), "Back to you · Mail");
    let rt = runtime(
        display(Some(back.clone()), false, None),
        frame::view,
        SCENE,
        false,
    );
    assert!(rect(&rt, "frame").is_none(), "no glow after");
    assert!(rect(&rt, "frame-back").is_some());
    assert!(back.shows(now + BACK_FOR / 2));
    assert!(!back.shows(now + BACK_FOR));
    assert_eq!(Tab::of(&board, now + BACK_FOR), None);
}

#[test]
fn a_take_over_ends_the_borrow_without_back_to_you() {
    let now = Instant::now();
    let mut board = borrowing(now, 20_000);
    board.apply(
        &Event::Paused {
            conversation: "export".into(),
        },
        None,
        now,
    );
    board.apply(
        &Event::Returned {
            conversation: "export".into(),
            to: None,
        },
        None,
        now,
    );
    assert_eq!(board.frame_until(now), None);
    assert_eq!(Tab::of(&board, now), None, "the dashed outline says it");
    assert_eq!(board.outlined()[0].look, Look::Paused);
}

#[test]
fn real_input_shows_the_frame_with_a_tab_naming_the_app() {
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    let mut board = Board::default();
    board.apply(
        &Event::Started {
            conversation: "quick".into(),
            origin: Origin::ActionShortcut,
            question: "Reply to Maya".into(),
        },
        None,
        now,
    );
    board.apply(
        &Event::Window {
            conversation: "quick".into(),
            window: TaskWindow {
                app: "Mail".into(),
                window_id: Some("w-2".into()),
            },
        },
        None,
        now,
    );
    assert_eq!(Tab::of(&board, now), None);
    board.apply(
        &Event::HandsOff {
            conversation: "quick".into(),
            on: true,
        },
        None,
        now,
    );
    let tab = Tab::of(&board, now).unwrap();
    assert_eq!(
        tab.line(),
        "Agent-Desk is using your mouse and keys in Mail"
    );
    assert_eq!(tab.detail(None).as_deref(), Some("Esc to stop"));
}

#[test]
fn why_says_each_reason_in_words() {
    crate::desktop::i18n::apply(Some("en"));
    for why in crate::assistant::guard::BORROW_REASONS {
        let words = frame::why_words(why, "Numbers");
        assert!(words.starts_with("Numbers "), "{why}: {words}");
    }
    assert_ne!(
        frame::why_words(crate::assistant::guard::BORROW_IGNORED, "Numbers"),
        frame::why_words(crate::assistant::guard::BORROW_KEYS, "Numbers")
    );
}

fn outline_of(look: Look, label: LabelSide) -> Outline {
    Outline {
        colour: 2,
        title: "Find a table for two at Nopa tonight".into(),
        size: (600.0, 400.0),
        label,
        look,
        progress: Some((2, 5)),
        step: Some("checking 8:15 pm".into()),
        here: false,
        origin: (0.0, 0.0),
    }
}

fn outline_size(label: LabelSide) -> (f32, f32) {
    let top = match label {
        LabelSide::Above => 0.0,
        LabelSide::Inside => 200.0,
    };
    let (r, _) = outline::rect((100.0, 200.0, 600.0, 400.0), top);
    (r[2], r[3])
}

const LOOKS: [(&str, Look); 4] = [
    ("working", Look::Working),
    ("paused", Look::Paused),
    (
        "waiting",
        Look::Waiting {
            until_ms: 1_760_000_000_000,
        },
    ),
    ("hands-off", Look::HandsOff),
];

#[test]
fn outline_label_names_the_task_in_every_look() {
    crate::desktop::i18n::apply(Some("en"));
    for (name, look) in LOOKS {
        for side in [LabelSide::Above, LabelSide::Inside] {
            let state = outline_of(look, side);
            let rt = runtime(state.clone(), outline::view, outline_size(side), false);
            assert_eq!(
                text_of(&rt, "label-text").as_deref(),
                Some(state.title.as_str()),
                "{name} {side:?}"
            );
            let words = text_of(&rt, "label-state").unwrap_or_default();
            match look {
                Look::Working | Look::HandsOff => {
                    assert_eq!(words, "· checking 8:15 pm · 2/5", "{name}")
                }
                Look::Paused => assert_eq!(words, "· you have it"),
                Look::Waiting { .. } => assert!(words.starts_with("· resumes "), "{words}"),
            }
            let [_, line_y, ..] = rect(&rt, "line").unwrap();
            let [lx, label_y, _, label_h] = rect(&rt, "label").unwrap();
            match side {
                LabelSide::Above => assert!(label_y + label_h <= line_y, "{name}: above"),
                LabelSide::Inside => {
                    assert!(
                        label_y > line_y && label_y < line_y + 20.0,
                        "{name}: inside top"
                    );
                    assert!(lx < outline::PAD + 20.0, "{name}: at the left");
                }
            }
        }
    }
}

#[test]
fn a_minis_ring_follows_what_its_job_does_there() {
    let now = Instant::now();
    let mut board = borrowing(now, 20_000);
    let task = |board: &Board| board.task("export").unwrap().clone();
    assert_eq!(Ring::of(&task(&board), now), Ring::Acting, "in front");
    board.apply(
        &Event::Returned {
            conversation: "export".into(),
            to: Some("Mail".into()),
        },
        None,
        now,
    );
    assert_eq!(Ring::of(&task(&board), now), Ring::Idle, "parked again");
    board.apply(
        &Event::Acting {
            conversation: "export".into(),
            looking: false,
        },
        None,
        now,
    );
    assert_eq!(Ring::of(&task(&board), now), Ring::Acting, "clicking");
    assert_eq!(
        Ring::of(&task(&board), now + Duration::from_secs(10)),
        Ring::Idle
    );
    board.task_mut("export").unwrap().phase = Phase::Paused;
    assert_eq!(Ring::of(&task(&board), now), Ring::Yours);
    crate::desktop::i18n::apply(Some("en"));
    assert_eq!(
        pictures::caption("Safari", Ring::Idle),
        "Safari · in background"
    );
    assert_eq!(
        pictures::spoken("Safari", "Book Nopa", Ring::Acting),
        "Safari, Book Nopa, acting"
    );
}

// ---------------------------------------------------------------- renders

/// The scene: a 1440 × 900 desktop with a menu bar and two windows.
const SCENE: (f32, f32) = (1440.0, 900.0);
const MENU_BAR: f32 = 37.0;
/// Numbers, brought in front, and Mail, the person's, behind it.
const NUMBERS_AT: [f32; 4] = [300.0, 170.0, 820.0, 540.0];
const MAIL_AT: [f32; 4] = [80.0, 90.0, 760.0, 560.0];

#[derive(Clone)]
struct Scene {
    frame: Frame,
    outline: Option<Outline>,
    /// Numbers is on the screen (parked again after "Back to you").
    numbers: bool,
}

fn window(p: Paint, app: &str, [x, y, w, h]: [f32; 4]) -> El {
    let c = &p.k.c;
    col()
        .key(format!("window-{app}"))
        .absolute(x, y)
        .size(w, h)
        .radius(10.0)
        .clip_rounded(10.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .box_shadow(look::shadow(p))
        .child(
            row()
                .h(30.0)
                .w(w)
                .px(14.0)
                .center_y()
                .fill(c.layer_02)
                .child(text(app.to_owned()).font(look::label(p)).color(p.muted())),
        )
        .child(col().padding(18.0).gap(10.0).children((0..7).map(|line| {
            el().h(10.0)
                .w(w * if line % 3 == 0 { 0.6 } else { 0.85 } - 36.0)
                .radius(5.0)
                .fill(c.border_base)
        })))
}

fn scene_view(scene: &Scene, cx: &mut Cx<Scene>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let desktop = if p.k.dark {
        rune::color::hex("#1d222b").unwrap_or(Color::BLACK)
    } else {
        rune::color::hex("#9fb3c8").unwrap_or(Color::WHITE)
    };
    let mut root = el()
        .key("scene")
        .size(SCENE.0, SCENE.1)
        .fill(desktop)
        .child(
            el().absolute(0.0, 0.0)
                .size(SCENE.0, MENU_BAR)
                .fill(p.k.c.bg_base.alpha(0.82)),
        )
        // The notch.
        .child(
            el().absolute(SCENE.0 / 2.0 - 98.0, 0.0)
                .size(196.0, MENU_BAR)
                .radius(12.0)
                .fill(Color::BLACK),
        )
        .child(window(p, "Mail", MAIL_AT));
    if scene.numbers {
        root = root.child(window(p, "Numbers", NUMBERS_AT));
    }
    if let Some(outline) = &scene.outline {
        let (at, _) = outline::rect(
            (
                f64::from(NUMBERS_AT[0]),
                f64::from(NUMBERS_AT[1]),
                f64::from(NUMBERS_AT[2]),
                f64::from(NUMBERS_AT[3]),
            ),
            f64::from(MENU_BAR),
        );
        root = root.child(
            el().absolute(at[0], at[1])
                .size(at[2], at[3])
                .child(outline::drawn(p, outline, 0.0, 0.0)),
        );
    }
    root.child(
        el().absolute(0.0, 0.0)
            .size(SCENE.0, SCENE.1)
            .child(frame::drawn(p, &scene.frame)),
    )
}

/// Three minis in their rings, as a job's row in the rail shows them.
fn minis_view(_: &(), cx: &mut Cx<()>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let colour = super::palette::task(0, p.k.dark);
    let picture = stand_in();
    let cell = |ring: Ring, name: &str| {
        col()
            .key(format!("mini-cell-{name}"))
            .w(jobs::THUMB_W)
            .gap(5.0)
            .child(pictures::ring(
                p,
                ring,
                colour,
                jobs::mini(
                    p,
                    format!("mini-{name}"),
                    &picture,
                    look::Liveness::Live,
                    (&super::super::icons::GLOBE, name),
                    true,
                ),
                (jobs::THUMB_W, jobs::THUMB_H),
            ))
            .child(
                text(pictures::caption(name, ring))
                    .truncate()
                    .font(look::meta(p))
                    .color(p.muted()),
            )
    };
    look::material(p, col().w(MINIS.0 - 24.0).padding(16.0).gap(8.0))
        .child(
            row().gap(8.0).child(look::dot(colour, 9.0)).child(
                text("Book a table at Nopa")
                    .font(look::label(p))
                    .color(p.text()),
            ),
        )
        .child(
            row()
                .gap(12.0)
                .pt(4.0)
                .child(cell(Ring::Acting, "Safari"))
                .child(cell(Ring::Idle, "Mail"))
                .child(cell(Ring::Yours, "Numbers")),
        )
}

const MINIS: (f32, f32) = (560.0, 200.0);

/// A picture standing in for a page.
fn stand_in() -> rune::Pixels {
    let (w, h) = (300u32, 188u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = if y < 26 {
                (230, 232, 236)
            } else if (40..110).contains(&y) && x > 16 && x < 284 {
                (40 + (x / 3) as u8, 140, 190)
            } else if y > 124 && y % 18 < 6 && x > 16 && x < 200 {
                (205, 208, 214)
            } else {
                (250, 250, 251)
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let pixels = rune::Pixels::new();
    pixels.set(w, h, rgba);
    pixels
}

/// `rt` drawn over a desktop grey, as the clear overlay sits on screen.
fn backdrop<S: 'static>(
    gpu: &rune::render::Gpu,
    rt: &mut Runtime<S>,
    (w, h): (u32, u32),
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

/// Renders the frames to `AGENT_DESK_ASSISTANT_SNAPS/assistant-*.png` in
/// both appearances: the desk frame with its tab (a borrow, real input,
/// "Back to you"), the outline in every look (and its chip inside), and
/// the minis' rings.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn frames_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    let in_front = Tab::of(&borrowing(now, 20_000), now).unwrap();
    let working = Outline {
        colour: 1,
        title: "Export the budget sheet as a PDF".into(),
        size: (NUMBERS_AT[2], NUMBERS_AT[3]),
        label: LabelSide::Above,
        look: Look::Working,
        progress: Some((3, 5)),
        step: Some("exporting PDF".into()),
        here: false,
        origin: (0.0, 0.0),
    };
    let scenes = [
        (
            "frame-borrowed",
            Scene {
                frame: display(Some(in_front.clone()), true, Some(2)),
                outline: Some(working.clone()),
                numbers: true,
            },
        ),
        (
            "frame-hands-off",
            Scene {
                frame: display(
                    Some(Tab::Hands {
                        app: Some("Numbers".into()),
                        window: None,
                    }),
                    true,
                    None,
                ),
                outline: Some(Outline {
                    look: Look::HandsOff,
                    here: true,
                    title: "Fill in the totals".into(),
                    progress: None,
                    step: Some("typing in C12".into()),
                    ..working.clone()
                }),
                numbers: true,
            },
        ),
        (
            "frame-back",
            Scene {
                frame: display(
                    Some(Tab::Back {
                        app: "Mail".into(),
                        until: now + BACK_FOR,
                    }),
                    false,
                    None,
                ),
                outline: None,
                numbers: false,
            },
        ),
    ];
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let save = |name: &str, size: (f32, f32), pixels: Vec<u8>| {
            let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
            let path = folder.join(format!("assistant-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        };
        let px = |size: (f32, f32)| ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        for (name, scene) in &scenes {
            let mut rt = runtime(scene.clone(), scene_view, SCENE, dark);
            save(name, SCENE, backdrop(&gpu, &mut rt, px(SCENE), dark));
        }
        for (name, look) in LOOKS {
            for (side, suffix) in [(LabelSide::Above, ""), (LabelSide::Inside, "-inside")] {
                if side == LabelSide::Inside && name != "working" {
                    continue;
                }
                let size = outline_size(side);
                let mut rt = runtime(outline_of(look, side), outline::view, size, dark);
                save(
                    &format!("outline-{name}{suffix}"),
                    size,
                    backdrop(&gpu, &mut rt, px(size), dark),
                );
            }
        }
        let mut rt = runtime((), minis_view, MINIS, dark);
        save(
            "mini-rings",
            MINIS,
            backdrop(&gpu, &mut rt, px(MINIS), dark),
        );
    }
}
