//! The overlays' views, laid out offscreen as the panels draw them, and
//! (ignored, with a GPU) rendered to PNGs to look at.

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};

use super::board::{Board, Phase};
use super::bubble::{self, BubbleOverlay};
use super::desk::{self, Desk};
use super::fixture;
use super::outline::{self, LabelSide, Outline};
use super::stack::{self, Stack};
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

fn stack_state(board: Board) -> Stack {
    Stack {
        board,
        shown: true,
        armed: None,
        actions: stack::Actions {
            stop: |_, _| {},
            open: |_, _| {},
        },
    }
}

fn bubble_state(bubble: Option<super::board::Bubble>) -> BubbleOverlay {
    BubbleOverlay {
        bubble,
        below: true,
        shown: true,
        actions: bubble::Actions {
            dismiss: |_| {},
            open: |_, _| {},
        },
    }
}

#[test]
fn the_stack_lists_each_task_with_its_state() {
    let board = fixture::board();
    let phases: Vec<Phase> = board.stack().iter().map(|task| task.phase).collect();
    assert_eq!(
        phases,
        [
            Phase::Running,
            Phase::Running,
            Phase::Queued,
            Phase::Done,
            Phase::Failed
        ]
    );
    for dark in [false, true] {
        let rt = runtime(stack_state(board.clone()), stack::view, stack::SIZE, dark);
        for task in board.stack() {
            let [x, y, w, h] = rect(&rt, &format!("task-{}", task.id))
                .unwrap_or_else(|| panic!("{} has a plate", task.id));
            assert!(x >= 0.0 && y >= 0.0, "inside: {x} {y}");
            assert!(
                x + w <= stack::SIZE.0 && y + h <= stack::SIZE.1,
                "inside: {w} {h}"
            );
        }
        assert_eq!(count(&rt, "spinner"), 2, "the running ones spin");
        assert_eq!(count(&rt, "queued"), 1);
        assert_eq!(count(&rt, "done"), 1);
        assert_eq!(count(&rt, "failed"), 1);
        assert_eq!(count(&rt, "stop"), 3, "only live tasks can be stopped");
        // The newest plate sits nearest the corner.
        let [_, last_y, _, last_h] = rect(&rt, "task-export").unwrap();
        assert!(last_y + last_h > stack::SIZE.1 - stack::INSET - 1.0);
    }
}

#[test]
fn a_long_stack_folds_its_oldest_tasks_into_a_count() {
    let now = std::time::Instant::now();
    let mut board = Board::default();
    for n in 0..9 {
        board.apply(
            &Event::Started {
                conversation: format!("t{n}"),
                origin: Origin::AgentMode,
                question: format!("Task number {n}"),
            },
            None,
            now,
        );
    }
    let rt = runtime(stack_state(board), stack::view, stack::SIZE, false);
    assert!(rect(&rt, "more").is_some(), "+3 more");
    assert!(rect(&rt, "task-t0").is_none(), "the oldest folded away");
    assert!(rect(&rt, "task-t8").is_some());
    let [_, top, ..] = rect(&rt, "more").unwrap();
    assert!(top >= 0.0, "the count stays in the overlay: {top}");
}

#[test]
fn an_empty_stack_draws_nothing() {
    let rt = runtime(
        stack_state(Board::default()),
        stack::view,
        stack::SIZE,
        false,
    );
    assert!(
        rt.elements()
            .iter()
            .all(|e| !e.key.as_deref().is_some_and(|k| k.starts_with("task-")))
    );
}

#[test]
fn the_bubble_shows_each_stage_of_a_quick_question() {
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
    let [_, card_y, _, _] = rect(&answered, "card").unwrap();
    assert!(
        (card_y - bubble::INSET).abs() < 0.5,
        "hangs from the top, under the HUD: {card_y}"
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
fn an_outline_frames_its_window_with_the_label_above() {
    let outline = Outline {
        colour: 2,
        title: "Find a table for two at Nopa tonight".into(),
        size: (600.0, 400.0),
        label: LabelSide::Above,
    };
    let (rect_global, _) = outline::rect((100.0, 200.0, 600.0, 400.0), 0.0);
    let size = (rect_global[2], rect_global[3]);
    let rt = runtime(outline, outline::view, size, false);
    let [lx, ly, lw, lh] = rect(&rt, "line").unwrap();
    assert_eq!((lw, lh), (600.0, 400.0));
    assert_eq!(lx, outline::PAD);
    let [_, label_y, _, label_h] = rect(&rt, "label").unwrap();
    assert!(label_y + label_h <= ly, "the label sits above the window");
}

#[test]
fn the_pretend_desktop_shows_every_overlay() {
    let rt = runtime(Desk::settled(), desk::view, desk::SIZE, true);
    for key in ["frame", "stack", "bubble", "outline-table", "outline-mail"] {
        assert!(rect(&rt, key).is_some(), "{key} draws");
    }
    assert!(
        rect(&rt, "outline-dentist").is_none(),
        "a finished task's outline is gone"
    );
}

/// Renders the overlays to `HEX_ASSISTANT_SNAPS/assistant-*.png` in both
/// appearances, to look at: the pretend desktop with everything, the stack,
/// and the bubble in each state.
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
        let mut desk_rt = runtime(Desk::settled(), desk::view, desk::SIZE, dark);
        let (w, h) = px(desk::SIZE);
        save(
            "desktop",
            desk::SIZE,
            crate::rune_ui::offscreen::draw(&gpu, &mut desk_rt, w, h),
        );
        let mut stack_rt = runtime(
            stack_state(fixture::board()),
            stack::view,
            stack::SIZE,
            dark,
        );
        let (w, h) = px(stack::SIZE);
        save(
            "stack",
            stack::SIZE,
            backdrop(&gpu, &mut stack_rt, w, h, dark),
        );
        let bubbles = [
            ("bubble-thinking", fixture::bubble_at(7.5)),
            ("bubble-working", fixture::bubble_at(9.1)),
            ("bubble-streaming", fixture::bubble_at(11.0)),
            ("bubble-answered", fixture::bubble_at(f64::MAX)),
            (
                "bubble-long",
                Some(fixture::bubble_ended(
                    "What changed in the release notes?",
                    Some(fixture::LONG_ANSWER),
                    None,
                )),
            ),
            (
                "bubble-failed",
                Some(fixture::bubble_ended(
                    fixture::QUICK_QUESTION,
                    None,
                    Some("Claude Code isn't signed in. Open Terminal and run “claude” to sign in."),
                )),
            ),
        ];
        for (name, bubble) in bubbles {
            let mut rt = runtime(bubble_state(bubble), bubble::view, bubble::SIZE, dark);
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
