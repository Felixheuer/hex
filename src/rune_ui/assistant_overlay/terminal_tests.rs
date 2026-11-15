//! A job's Terminal mini on its card and the terminal large, laid out
//! offscreen, and (ignored, with a GPU) rendered to
//! `assistant-terminal-{small,large}-{light,dark}.png`.

use std::time::{Duration, Instant};

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime};
use serde_json::json;

use super::super::board::Board;
use super::super::enlarged::{self, Enlarged};
use super::super::orb::{self, Orb};
use super::super::{fixture, live_card};
use super::*;
use crate::assistant::view::{ToolUse, console};
use crate::assistant::{Event, Origin};

const JOB: &str = "improve";

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

fn texts<S>(rt: &Runtime<S>, prefix: &str) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
        .filter_map(|e| e.text)
        .collect()
}

fn tool(name: &str, input: serde_json::Value) -> Event {
    let shown = ToolUse::of(name, &input);
    Event::Tool {
        conversation: JOB.into(),
        lines: console::call_lines(shown.kind, &input),
        tool: shown,
    }
}

fn done(output: &str) -> Event {
    Event::ToolDone {
        conversation: JOB.into(),
        lines: console::output_lines(ToolKind::Command, output),
    }
}

/// A board with one job that has just started; `work` adds its tool calls.
fn board(work: bool, at: Instant) -> Board {
    let mut board = Board::default();
    board.apply(
        &Event::Started {
            conversation: JOB.into(),
            origin: Origin::Background,
            question: "Verbessere dich iterativ".into(),
        },
        Some(1),
        at,
    );
    if !work {
        return board;
    }
    for event in [
        tool(
            "Grep",
            json!({"pattern": "Kontakte verknüpfen", "description": "contacts"}),
        ),
        tool("Read", json!({"file_path": "/repo/src/setup_gaps.rs"})),
        tool(
            "Edit",
            json!({
                "file_path": "/repo/src/setup_gaps.rs",
                "old_string": "let gaps = scan();",
                "new_string": "let gaps = scan_all();\nlog(gaps.len());",
            }),
        ),
        tool(
            "Bash",
            json!({"command": "OPENAI_API_KEY=sk-test123456 cargo test setup_gaps", "description": "Run tests"}),
        ),
        done("running 4 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 4 passed"),
        tool("Bash", json!({"command": "cargo build --release"})),
    ] {
        board.apply(&event, None, at);
    }
    board
}

fn orb_with(board: Board) -> Orb {
    let mut orb = Orb::new(board, fixture::ANSWER_KEY.into(), orb::Actions::inert());
    orb.shown = true;
    orb
}

fn enlarged_with(board: &Board) -> Enlarged {
    let mut state = Enlarged::new(live_card::Actions {
        close: |s| {
            enlarged::close(s);
        },
        take_over: |_, _| {},
        hand_back: |_, _| {},
        what_it_did: None,
    });
    enlarged::open(&mut state, JOB, SESSION, board);
    state
}

#[test]
fn the_terminal_mini_shows_only_once_a_job_has_activity() {
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    let quiet = runtime(orb_with(board(false, now)), orb::view, orb::SIZE, false);
    assert!(rect(&quiet, &format!("job-{JOB}")).is_some(), "its card");
    assert!(
        rect(&quiet, &format!("job-term-cell-{JOB}")).is_none(),
        "no terminal before it runs anything"
    );
    let busy = runtime(orb_with(board(true, now)), orb::view, orb::SIZE, false);
    let mini = rect(&busy, &format!("job-term-{JOB}")).expect("the mini");
    assert_eq!((mini[2], mini[3]), (THUMB_W, THUMB_H));
    // Its newest lines, as many as fit, the cursor under the last.
    assert!(MINI_LINES as f32 * MINI_LINE_H <= THUMB_H - 2.0 * MINI_PAD);
    let lines = texts(&busy, "term-line-");
    assert_eq!(lines.len(), MINI_LINES - 1, "{lines:?}");
    assert!(
        lines.contains(&"+ log(gaps.len());".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"test result: ok. 4 passed".to_owned()));
    assert_eq!(
        lines.last().map(String::as_str),
        Some("$ cargo build --release")
    );
    let cursor = rect(&busy, "term-cursor").expect("a command runs");
    assert!(
        cursor[1] + cursor[3] <= mini[1] + mini[3],
        "inside: {cursor:?}"
    );
    assert!(
        lines.iter().all(|line| !line.contains("sk-test123456")),
        "no secret on the screen: {lines:?}"
    );
    // The card's line says the same, in words.
    assert!(
        texts(&busy, "job-step")
            .iter()
            .any(|line| line.contains("Running a command")),
        "{:?}",
        texts(&busy, "job-step")
    );
}

#[test]
fn the_terminal_opens_large_with_its_whole_log_and_follows_the_board() {
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    let mut board = board(true, now);
    let mut state = enlarged_with(&board);
    let shown = state.terminal.clone().expect("the terminal");
    assert_eq!(shown.console.len(), board.tasks[0].console.len());
    assert!(shown.running.is_some(), "the build runs");
    // A new line reaches it with the board, nothing polled.
    board.apply(&done("Finished release"), None, now);
    assert!(enlarged::follow(&mut state, &board));
    assert!(!enlarged::follow(&mut state, &board), "nothing new");
    assert_eq!(state.terminal.as_ref().unwrap().running, None);
    let rt = runtime(state, enlarged::view, enlarged::SIZE, true);
    assert!(rect(&rt, "term-card").is_some());
    let log = rect(&rt, "term-log").expect("its log");
    assert!(log[3] > 300.0, "large: {log:?}");
    let lines = texts(&rt, "term-line-");
    assert_eq!(lines.last().map(String::as_str), Some("Finished release"));
    for line in [
        "⌕ searching \"Kontakte verknüpfen\"",
        "▤ reading setup_gaps.rs",
        "✎ edited setup_gaps.rs (+2 −1)",
        "− let gaps = scan();",
        "+ let gaps = scan_all();",
    ] {
        assert!(lines.contains(&line.to_owned()), "{line} in {lines:?}");
    }
    assert!(
        lines.contains(&"$ OPENAI_API_KEY=••• cargo test setup_gaps".to_owned()),
        "{lines:?}"
    );
}

#[test]
fn a_long_command_shows_its_time_in_the_mini() {
    crate::desktop::i18n::apply(Some("en"));
    let then = Instant::now();
    let board = board(true, then);
    let task = &board.tasks[0];
    let at = |secs| then + Duration::from_secs(secs);
    let shown = |now| {
        lines(
            &task.console.tail(2),
            running(task),
            look::meta(Paint::new(false, 2.0)),
            true,
            now,
        )
        .len()
    };
    assert_eq!(shown(at(1)), 3, "two lines and the cursor");
    assert!(running(task).is_some());
    assert_eq!(
        task.busy_line(at(80)).as_deref(),
        Some("Running a command · 1:20")
    );
}

/// Renders the mini on its card and the terminal large, light and dark,
/// to `AGENT_DESK_ASSISTANT_SNAPS`.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn terminal_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    crate::desktop::i18n::apply(Some("en"));
    let now = Instant::now();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let size = orb::SIZE;
        let mut rt = runtime(orb_with(board(true, now)), orb::view, size, dark);
        let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h);
        let path = folder.join(format!("assistant-terminal-small-{theme}.png"));
        rune::render::write_png(&path, w, h, &pixels).unwrap();
        let size = enlarged::SIZE;
        let mut rt = runtime(enlarged_with(&board(true, now)), enlarged::view, size, dark);
        let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h);
        let path = folder.join(format!("assistant-terminal-large-{theme}.png"));
        rune::render::write_png(&path, w, h, &pixels).unwrap();
    }
}
