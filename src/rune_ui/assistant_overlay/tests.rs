//! The overlays' views, laid out offscreen as the panels draw them, and
//! (ignored, with a GPU) rendered to PNGs to look at.

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};

use super::board::{Board, Look, Phase};
use super::bubble::{self, BubbleOverlay};
use super::desk::{self, Desk};
use super::fixture;
use super::hint::{self, Hint};
use super::orb::{self, Orb};
use super::outline::{self, LabelSide, Outline};
use super::{hands, stack};
use crate::assistant::{Event, Origin};

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

fn count<S>(rt: &Runtime<S>, key: &str) -> usize {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some(key))
        .count()
}

fn count_prefix<S>(rt: &Runtime<S>, prefix: &str) -> usize {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
        .count()
}

fn inside(r: [f32; 4], size: (f32, f32)) -> bool {
    let [x, y, w, h] = r;
    x >= -0.5 && y >= -0.5 && x + w <= size.0 + 0.5 && y + h <= size.1 + 0.5
}

fn orb_state(board: Board, expanded: bool) -> Orb {
    Orb {
        board,
        shown: true,
        armed: None,
        expanded,
        peek: false,
        grow: orb::Grow::default(),
        answer_key: fixture::ANSWER_KEY.into(),
        actions: orb::Actions {
            list: stack::Actions {
                stop: |_, _| {},
                open: |_, _| {},
                take_over: |_, _| {},
                hand_back: |_, _| {},
                stop_all: |_| {},
                toggle: |_| {},
                dismiss: |_, _| {},
            },
            hover: |_, _| {},
            drag: |_, _| {},
        },
    }
}

fn bubble_state(bubble: Option<super::board::Bubble>) -> BubbleOverlay {
    BubbleOverlay {
        bubble,
        below: true,
        shown: true,
        speaking: false,
        setup: None,
        answer_key: fixture::ANSWER_KEY.into(),
        actions: bubble::Actions {
            dismiss: |_| {},
            open: |_, _| {},
            setup: |_| {},
        },
    }
}

#[test]
fn the_fixture_has_a_task_in_every_state() {
    let phases: Vec<Phase> = fixture::board()
        .stack()
        .iter()
        .map(|task| task.phase)
        .collect();
    assert!(matches!(
        phases.as_slice(),
        [
            Phase::Running,
            Phase::Paused,
            Phase::Queued,
            Phase::Done,
            Phase::Failed,
            Phase::Waiting { .. },
            Phase::NeedsInput
        ]
    ));
}

#[test]
fn the_orb_shows_a_dot_per_live_task_and_its_bubbles() {
    let board = fixture::board();
    for dark in [false, true] {
        let rt = runtime(orb_state(board.clone(), false), orb::view, orb::SIZE, dark);
        let core = rect(&rt, "core").expect("the orb's core");
        assert!(core[0] < 40.0 && core[1] < 40.0, "in its corner: {core:?}");
        assert_eq!(
            count_prefix(&rt, "orb-dot-"),
            5,
            "running, taken over, queued, waiting, asking"
        );
        assert_eq!(count(&rt, "ring"), 1, "the table task's plan");
        let asked = rect(&rt, "question-invoices").expect("its question");
        assert!(inside(asked, orb::SIZE), "{asked:?}");
        assert!(rect(&rt, "question-hint").is_some(), "Hold ⌘ to answer");
        let [_, asked_y, ..] = asked;
        let [_, paused_y, ..] = rect(&rt, "standing-mail").unwrap();
        assert!(asked_y < paused_y, "questions sit nearest the orb");
        assert!(rect(&rt, "standing-mail").is_some(), "You have it");
        assert!(rect(&rt, "standing-news").is_some(), "Waiting for limit");
        assert_eq!(count(&rt, "hand-back"), 1);
        assert!(count_prefix(&rt, "pop-") >= 1, "bubbles about the tasks");
        assert!(rect(&rt, "sheet").is_none(), "the list stays folded");
        for key in ["orb", "standing-mail", "standing-news"] {
            assert!(inside(rect(&rt, key).unwrap(), orb::SIZE), "{key} inside");
        }
    }
}

#[test]
fn a_quiet_orb_has_no_bubbles_and_no_tasks_draw_nothing() {
    let rt = runtime(
        orb_state(fixture::quiet_board(), false),
        orb::view,
        orb::SIZE,
        false,
    );
    assert!(rect(&rt, "orb").is_some());
    assert_eq!(count_prefix(&rt, "pop-"), 0);
    let empty = runtime(
        orb_state(Board::default(), false),
        orb::view,
        orb::SIZE,
        false,
    );
    assert!(rect(&empty, "orb").is_none());
}

#[test]
fn the_list_unfolds_with_every_task_its_state_and_controls() {
    let board = fixture::board();
    for dark in [false, true] {
        let rt = runtime(orb_state(board.clone(), true), orb::view, orb::SIZE, dark);
        let sheet = rect(&rt, "sheet").expect("the list");
        assert!(inside(sheet, orb::SIZE), "{sheet:?}");
        assert_eq!(count_prefix(&rt, "task-"), 7);
        assert_eq!(count(&rt, "spinner"), 1, "the running one spins");
        assert_eq!(count(&rt, "needs-input"), 1);
        assert_eq!(count(&rt, "plan-current"), 1, "the step it's on");
        assert_eq!(count(&rt, "plan-text"), 5, "its whole plan");
        assert_eq!(count(&rt, "paused"), 1, "You have it");
        assert_eq!(count(&rt, "countdown"), 1, "minutes until the limit lifts");
        assert_eq!(count(&rt, "queued"), 1);
        assert_eq!(count(&rt, "done"), 1);
        assert_eq!(count(&rt, "failed"), 1);
        assert_eq!(count(&rt, "take-over"), 1);
        assert_eq!(count(&rt, "hand-back"), 1);
        assert_eq!(count(&rt, "stop"), 5, "only live tasks can be stopped");
        assert!(rect(&rt, "stop-all").is_some(), "two or more live tasks");
        assert!(count(&rt, "place-glyph") >= 3, "where each works");
        assert_eq!(
            count_prefix(&rt, "pop-"),
            0,
            "the list replaces the bubbles"
        );
        // Newest nearest the orb, below it.
        let [_, news_y, ..] = rect(&rt, "task-news").unwrap();
        let [_, table_y, ..] = rect(&rt, "task-table").unwrap();
        assert!(news_y < table_y);
    }
}

#[test]
fn an_escalated_question_flies_into_the_orb() {
    let rt = runtime(
        orb_state(fixture::escalated_board(), false),
        orb::view,
        orb::SIZE,
        false,
    );
    assert!(rect(&rt, &format!("orb-dot-{}", fixture::ESCALATED)).is_some());
    assert_eq!(count(&rt, "pop-text"), 1, "its title pops");
    let mut flying = fixture::bubble_ended("Sort my Downloads", None, None);
    flying.escalated = true;
    assert_eq!(
        flying.due(std::time::Instant::now(), false),
        super::board::Due::Leave
    );
    let rt = runtime(
        bubble_state(Some(flying)),
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&rt, "card").is_some(), "drawn once as it flies");
}

#[test]
fn a_here_question_asks_in_its_bubble() {
    let mut asking = fixture::bubble_at(9.1).unwrap();
    asking.asks = Some("Reply to Maya or to the whole thread?".into());
    let rt = runtime(
        bubble_state(Some(asking)),
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&rt, "asks-text").is_some());
    assert!(rect(&rt, "asks-hint").is_some());
}

#[test]
fn stop_all_needs_two_live_tasks() {
    let now = std::time::Instant::now();
    let mut board = Board::default();
    board.apply(
        &Event::Started {
            conversation: "one".into(),
            origin: Origin::Background,
            question: "Only task".into(),
        },
        None,
        now,
    );
    let rt = runtime(orb_state(board, true), orb::view, orb::SIZE, false);
    assert!(rect(&rt, "task-one").is_some());
    assert!(rect(&rt, "stop-all").is_none());
}

#[test]
fn a_long_list_scrolls_inside_the_panel() {
    let now = std::time::Instant::now();
    let mut board = Board::default();
    for n in 0..9 {
        board.apply(
            &Event::Started {
                conversation: format!("t{n}"),
                origin: Origin::Background,
                question: format!("Task number {n}"),
            },
            None,
            now,
        );
        for step in ["Opened Safari", "Read the page", "Typed a reply"] {
            board.apply(
                &Event::Step {
                    conversation: format!("t{n}"),
                    description: step.into(),
                },
                None,
                now,
            );
        }
    }
    let rt = runtime(orb_state(board, true), orb::view, orb::SIZE, false);
    let sheet = rect(&rt, "sheet").unwrap();
    assert!(inside(sheet, orb::SIZE), "{sheet:?}");
}

#[test]
fn the_bubble_shows_each_stage_of_a_here_question() {
    let thinking = runtime(
        bubble_state(fixture::bubble_at(7.5)),
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&thinking, "thinking").is_some());
    assert!(
        rect(&thinking, "open-chats").is_none(),
        "nothing to open yet"
    );

    let working = runtime(
        bubble_state(fixture::bubble_at(9.1)),
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&working, "done-0").is_some(), "the earlier step");
    assert!(rect(&working, "step").is_some(), "the current one spins");

    let answered = runtime(
        bubble_state(fixture::bubble_at(f64::MAX)),
        bubble::view,
        bubble::SIZE,
        true,
    );
    assert!(rect(&answered, "answer").is_some());
    assert!(rect(&answered, "open-chats").is_some());
    assert!(rect(&answered, "voice").is_none(), "silent");
    let [_, card_y, _, _] = rect(&answered, "card").unwrap();
    assert!(
        (card_y - bubble::INSET).abs() < 0.5,
        "hangs from the top, under the HUD: {card_y}"
    );

    let speaking = runtime(
        BubbleOverlay {
            speaking: true,
            ..bubble_state(fixture::bubble_at(f64::MAX))
        },
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(
        rect(&speaking, "voice").is_some(),
        "pulses while read aloud"
    );

    let failed = runtime(
        bubble_state(Some(fixture::bubble_ended(
            "Hello?",
            None,
            Some("Claude Code isn't signed in."),
        ))),
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&failed, "error").is_some());

    let hidden = runtime(
        BubbleOverlay {
            shown: false,
            ..bubble_state(fixture::bubble_at(f64::MAX))
        },
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&hidden, "card").is_none(), "shrinks back as it hides");
}

#[test]
fn before_setup_the_bubble_offers_to_continue_it() {
    let rt = runtime(
        BubbleOverlay {
            setup: Some(std::time::Instant::now()),
            ..bubble_state(None)
        },
        bubble::view,
        bubble::SIZE,
        false,
    );
    assert!(rect(&rt, "setup-text").is_some());
    let button = rect(&rt, "continue-setup").unwrap();
    assert!(inside(button, bubble::SIZE));
}

#[test]
fn a_long_answer_stays_inside_the_bubble() {
    let long = fixture::bubble_ended("What changed?", Some(fixture::LONG_ANSWER), None);
    for below in [true, false] {
        let rt = runtime(
            BubbleOverlay {
                below,
                ..bubble_state(Some(long.clone()))
            },
            bubble::view,
            bubble::SIZE,
            false,
        );
        let [x, y, w, h] = rect(&rt, "card").unwrap();
        assert!(x >= 0.0 && y >= 0.0, "{x} {y}");
        assert!(
            x + w <= bubble::SIZE.0 && y + h <= bubble::SIZE.1 + 0.5,
            "{w} {h}"
        );
        if !below {
            assert!(
                (y + h - (bubble::SIZE.1 - bubble::INSET)).abs() < 0.5,
                "rises from the bottom, above a HUD at the bottom"
            );
        }
    }
}

#[test]
fn no_bubble_draws_nothing() {
    let rt = runtime(bubble_state(None), bubble::view, bubble::SIZE, false);
    assert!(rect(&rt, "card").is_none());
}

#[test]
fn the_hints_and_the_hands_off_pill_fit_their_panels() {
    for state in [Hint::Background, Hint::Here("Microsoft Outlook".into())] {
        let here = matches!(state, Hint::Here(_));
        let rt = runtime(state, hint::view, hint::SIZE, false);
        let key = if here { "here" } else { "hint" };
        assert!(inside(rect(&rt, key).unwrap(), hint::SIZE), "{key}");
    }
    let rt = runtime(hands::Hands::default(), hands::view, hands::SIZE, true);
    let pill = rect(&rt, "hands-off").unwrap();
    assert!(inside(pill, hands::SIZE), "{pill:?}");
    assert!(rect(&rt, "hands-esc").is_some());
}

fn outline_of(look: Look) -> Runtime<Outline> {
    let outline = Outline {
        colour: 2,
        title: "Find a table for two at Nopa tonight".into(),
        size: (600.0, 400.0),
        label: LabelSide::Above,
        look,
        progress: None,
        origin: (0.0, 0.0),
    };
    let (rect_global, _) = outline::rect((100.0, 200.0, 600.0, 400.0), 0.0);
    runtime(
        outline,
        outline::view,
        (rect_global[2], rect_global[3]),
        false,
    )
}

#[test]
fn an_outline_frames_its_window_with_the_label_above() {
    let rt = outline_of(Look::Working);
    let [lx, ly, lw, lh] = rect(&rt, "line").unwrap();
    assert_eq!((lw, lh), (600.0, 400.0));
    assert_eq!(lx, outline::PAD);
    let [_, label_y, _, label_h] = rect(&rt, "label").unwrap();
    assert!(label_y + label_h <= ly, "the label sits above the window");
    for look in [Look::Paused, Look::Waiting { until_ms: 0 }] {
        assert!(rect(&outline_of(look), "label").is_some(), "{look:?}");
    }
    let soft = outline_of(Look::HandsOff);
    assert!(rect(&soft, "line").is_some());
    assert!(
        rect(&soft, "label").is_none(),
        "Hands off is just a soft line"
    );
}

#[test]
fn the_pretend_desktop_shows_every_overlay() {
    let rt = runtime(Desk::settled(), desk::view, desk::SIZE, true);
    for key in [
        "frame",
        "orb",
        "bubble",
        "outline-table",
        "outline-mail",
        "outline-news",
    ] {
        assert!(rect(&rt, key).is_some(), "{key} draws");
    }
    assert!(
        rect(&rt, "outline-dentist").is_none(),
        "a finished task's outline is gone"
    );
    let [orb_x, ..] = rect(&rt, "orb").unwrap();
    let [bubble_x, _, bubble_w, _] = rect(&rt, "bubble").unwrap();
    assert!(
        orb_x > bubble_x + bubble_w,
        "the orb keeps clear of the Here bubble"
    );
    let hands = runtime(hands_off_desk(), desk::view, desk::SIZE, false);
    assert!(rect(&hands, "hands-off").is_some());
    assert!(rect(&hands, "outline-quick").is_some(), "its soft outline");
}

#[test]
fn on_two_displays_the_frame_joins_and_the_orb_is_on_the_working_one() {
    let scene = super::multi::TwoDisplays {
        board: fixture::quiet_board(),
    };
    let rt = runtime(scene, super::multi::view, super::multi::SIZE, true);
    assert_eq!(
        count(&rt, "frame-joined"),
        2,
        "a frame on each display, joined"
    );
    // The M27Q glows on where it overhangs the built-in display.
    assert_eq!(count_prefix(&rt, "frame-open-Bottom"), 2);
    for app in ["Safari", "Mail", "Arc"] {
        assert!(rect(&rt, &format!("outline-{app}")).is_some(), "{app}");
    }
    let [ox, oy, ..] = rect(&rt, "orb").expect("the orb");
    let [mx, my, mw, mh] = rect(&rt, "display-m27q").unwrap();
    assert!(
        ox >= mx && ox < mx + mw && oy >= my && oy < my + mh,
        "on the M27Q, where the person works"
    );
}

#[test]
fn a_frame_alone_is_the_whole_ring_and_a_joined_one_moves_off_its_seam() {
    use super::frame::{self, Frame};
    let alone = runtime(
        Frame {
            display: Some(1),
            size: (1800.0, 1169.0),
            seams: Default::default(),
        },
        frame::view,
        (1800.0, 1169.0),
        false,
    );
    assert_eq!(rect(&alone, "frame"), Some([0.0, 0.0, 1800.0, 1169.0]));
    let all = super::multi::screens();
    let built_in = runtime(
        Frame {
            display: Some(1),
            size: (1800.0, 1169.0),
            seams: super::screens::seams(&all[0], &all),
        },
        frame::view,
        (1800.0, 1169.0),
        false,
    );
    let [_, y, _, h] = rect(&built_in, "frame").unwrap();
    assert!(y < -100.0, "its top glow sits past the seam: {y}");
    assert!((y + h - 1169.0).abs() < 0.5, "the other edges stay");
}

/// The desktop while the Here question holds the pointer.
fn hands_off_desk() -> Desk {
    Desk {
        board: fixture::board_at(9.0),
        bubble: fixture::bubble_at(9.0),
        ..Desk::settled()
    }
}

/// Renders the overlays to `HEX_ASSISTANT_SNAPS/assistant-*.png` in both
/// appearances, to look at: the pretend desktop in each state, the orb, the
/// list, the bubble in each state, the hints, and the Hands off pill.
#[test]
#[ignore = "set HEX_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("HEX_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let save = |name: &str, size: (f32, f32), pixels: Vec<u8>| {
            let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
            let path = folder.join(format!("assistant-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        };
        let px = |size: (f32, f32)| ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        let desks = [
            ("desktop", Desk::settled()),
            (
                "desktop-list",
                Desk {
                    expanded: true,
                    bubble: None,
                    ..Desk::settled()
                },
            ),
            ("desktop-hands-off", hands_off_desk()),
            (
                "desktop-new-background-task",
                Desk {
                    hint: Some(Hint::Background),
                    ..Desk::settled()
                },
            ),
            (
                "desktop-here",
                Desk {
                    hint: Some(Hint::Here("Mail".into())),
                    board: fixture::quiet_board(),
                    ..Desk::settled()
                },
            ),
            (
                "desktop-speaking",
                Desk {
                    speaking: true,
                    board: fixture::quiet_board(),
                    ..Desk::settled()
                },
            ),
            (
                "desktop-escalated",
                Desk {
                    board: fixture::escalated_board(),
                    bubble: None,
                    ..Desk::settled()
                },
            ),
            (
                "desktop-setup",
                Desk {
                    setup: true,
                    frame: false,
                    board: Board::default(),
                    ..Desk::settled()
                },
            ),
        ];
        // The whole desk at 1×: 2560 × 2609 pixels.
        let mut two = crate::rune_ui::offscreen::runtime(
            super::multi::TwoDisplays {
                board: fixture::board(),
            },
            super::multi::view,
            super::multi::SIZE,
            Env {
                system_dark: dark,
                scale: 1.0,
            },
            TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
        );
        let (tw, th) = (super::multi::SIZE.0 as u32, super::multi::SIZE.1 as u32);
        let path = folder.join(format!("assistant-two-displays-{theme}.png"));
        let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut two, tw, th);
        rune::render::write_png(&path, tw, th, &pixels).unwrap();
        let (w, h) = px(desk::SIZE);
        for (name, state) in desks {
            let mut rt = runtime(state, desk::view, desk::SIZE, dark);
            save(
                name,
                desk::SIZE,
                crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h),
            );
        }
        let orbs = [
            ("orb-bubbles", orb_state(fixture::board(), false)),
            ("orb-quiet", orb_state(fixture::quiet_board(), false)),
            ("orb-list", orb_state(fixture::board(), true)),
            (
                "orb-escalated",
                orb_state(fixture::escalated_board(), false),
            ),
        ];
        for (name, state) in orbs {
            let mut rt = runtime(state, orb::view, orb::SIZE, dark);
            let (w, h) = px(orb::SIZE);
            save(name, orb::SIZE, backdrop(&gpu, &mut rt, w, h, dark));
        }
        for look in [
            ("outline-paused", Look::Paused),
            (
                "outline-waiting",
                Look::Waiting {
                    until_ms: super::when::now_ms() + 23 * 60_000,
                },
            ),
            ("outline-hands-off", Look::HandsOff),
        ] {
            let mut rt = outline_of(look.1);
            let (r, _) = outline::rect((100.0, 200.0, 600.0, 400.0), 0.0);
            let size = (r[2], r[3]);
            let (w, h) = px(size);
            save(look.0, size, backdrop(&gpu, &mut rt, w, h, dark));
        }
        let mut hands_rt = runtime(hands::Hands::default(), hands::view, hands::SIZE, dark);
        let (w, h) = px(hands::SIZE);
        save(
            "hands-off",
            hands::SIZE,
            backdrop(&gpu, &mut hands_rt, w, h, dark),
        );
        #[cfg(target_os = "macos")]
        if dark {
            for index in 0..super::palette::COLOURS {
                let pixels = super::cursor::render(&gpu, index);
                save(&format!("cursor-{index}"), super::cursor::SIZE, pixels);
            }
        }
        for (name, state) in [
            ("hint-background", Hint::Background),
            ("hint-here", Hint::Here("Mail".into())),
        ] {
            let mut rt = runtime(state, hint::view, hint::SIZE, dark);
            let (w, h) = px(hint::SIZE);
            save(name, hint::SIZE, backdrop(&gpu, &mut rt, w, h, dark));
        }
        let mut asking = fixture::bubble_at(9.1).unwrap();
        asking.asks = Some("Reply to Maya, or to the whole thread?".into());
        let bubbles = [
            ("bubble-question", bubble_state(Some(asking))),
            ("bubble-thinking", bubble_state(fixture::bubble_at(7.5))),
            ("bubble-working", bubble_state(fixture::bubble_at(9.1))),
            ("bubble-streaming", bubble_state(fixture::bubble_at(11.0))),
            (
                "bubble-speaking",
                BubbleOverlay {
                    speaking: true,
                    ..bubble_state(fixture::bubble_at(f64::MAX))
                },
            ),
            (
                "bubble-markdown",
                bubble_state(Some(fixture::bubble_ended(
                    "How do I free up disk space?",
                    Some(fixture::MARKDOWN_ANSWER),
                    None,
                ))),
            ),
            (
                "bubble-setup",
                BubbleOverlay {
                    setup: Some(std::time::Instant::now()),
                    ..bubble_state(None)
                },
            ),
        ];
        for (name, state) in bubbles {
            let mut rt = runtime(state, bubble::view, bubble::SIZE, dark);
            let (w, h) = px(bubble::SIZE);
            save(name, bubble::SIZE, backdrop(&gpu, &mut rt, w, h, dark));
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
