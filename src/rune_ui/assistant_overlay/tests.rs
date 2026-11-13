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

fn text_of<S>(rt: &Runtime<S>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .and_then(|e| e.text)
}

fn inside(r: [f32; 4], size: (f32, f32)) -> bool {
    let [x, y, w, h] = r;
    x >= -0.5 && y >= -0.5 && x + w <= size.0 + 0.5 && y + h <= size.1 + 0.5
}

fn orb_state(board: Board, expanded: bool) -> Orb {
    let mut orb = Orb::new(board, fixture::ANSWER_KEY.into(), orb::Actions::inert());
    orb.shown = true;
    orb.expanded = expanded;
    orb
}

fn bubble_state(bubble: Option<super::board::Bubble>) -> BubbleOverlay {
    BubbleOverlay {
        bubble,
        below: true,
        shown: true,
        speaking: false,
        setup: None,
        answer_key: fixture::ANSWER_KEY.into(),
        level: None,
        actions: bubble::Actions {
            dismiss: |_| {},
            open: |_, _| {},
            setup: |_| {},
            settings: |_| {},
            retry: |_, _, _| {},
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
fn the_rail_head_counts_its_jobs_in_words_and_its_cards_follow() {
    let board = fixture::board();
    // A panel as short as it gets on a small display, below the middle.
    let size = (orb::SIZE.0, 600.0);
    for dark in [false, true] {
        let mut state = orb_state(board.clone(), false);
        state.room = size.1;
        let rt = runtime(state, orb::view, size, dark);
        let core = rect(&rt, "core").expect("the orb's core");
        assert!(core[0] < 40.0 && core[1] < 40.0, "in its corner: {core:?}");
        assert_eq!(
            text_of(&rt, "orb-count").as_deref(),
            Some("5 jobs"),
            "running, taken over, queued, waiting, asking: in words, no dots"
        );
        assert_eq!(count_prefix(&rt, "orb-dot-"), 0, "no dot cluster");
        assert_eq!(count(&rt, "ring"), 1, "the table task's plan, at its row");
        let asked = rect(&rt, "question-invoices").expect("its question");
        assert!(inside(asked, size), "{asked:?}");
        assert!(rect(&rt, "question-hint").is_some(), "Hold ⌘ to answer");
        let [_, asked_y, ..] = asked;
        let [_, paused_y, ..] = rect(&rt, "standing-mail").unwrap();
        assert!(asked_y < paused_y, "questions sit nearest the orb");
        assert!(rect(&rt, "standing-mail").is_some(), "You have it");
        assert!(rect(&rt, "standing-news").is_some(), "Waiting for limit");
        assert_eq!(count(&rt, "hand-back"), 1);
        // Beside the question, three tasks, and a row, one result has room:
        // the failure (a done one gives way first; the list recalls it).
        assert_eq!(count(&rt, "result-head"), 1, "one result beside it all");
        assert_eq!(count(&rt, "result-retry"), 1, "the failure stays");
        assert_eq!(count(&rt, "pop-text"), 0, "no toasts while jobs work");
        assert!(rect(&rt, "sheet").is_none(), "the list stays folded");
        for key in ["orb", "standing-mail", "standing-news"] {
            assert!(inside(rect(&rt, key).unwrap(), size), "{key} inside");
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
        assert_eq!(count_prefix(&rt, "task-"), 5, "the live ones");
        assert_eq!(count_prefix(&rt, "result-row-"), 2, "and the results");
        assert_eq!(count(&rt, "chip-working"), 1, "the running one says so");
        assert_eq!(count(&rt, "spinner"), 0, "nothing spins");
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
            count(&rt, "result-head"),
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
    assert!(rect(&rt, "orb-count").is_some(), "the head counts it");
    // A running job shows as its card (title there), not as a "New" pop.
    assert!(
        rect(&rt, &format!("job-{}", fixture::ESCALATED)).is_some(),
        "its card shows"
    );
    assert_eq!(count(&rt, "pop-text"), 0, "no second bubble for it");
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
        step: Some("checking 8:15 pm".into()),
        here: false,
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
    for look in [Look::Paused, Look::Waiting { until_ms: 0 }, Look::HandsOff] {
        assert!(rect(&outline_of(look), "label").is_some(), "{look:?}");
    }
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
            glow: true,
            ..Default::default()
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
            glow: true,
            ..Default::default()
        },
        frame::view,
        (1800.0, 1169.0),
        false,
    );
    let [_, y, _, h] = rect(&built_in, "frame").unwrap();
    assert!(y < -100.0, "its top glow sits past the seam: {y}");
    assert!((y + h - 1169.0).abs() < 0.5, "the other edges stay");
}

#[test]
fn listening_glows_with_its_countdown_and_its_label_fades() {
    use super::listening::{self, Listening};
    let now = std::time::Instant::now();
    let fresh = runtime(
        Listening::start(now, 8.0),
        listening::view,
        listening::SIZE,
        true,
    );
    assert!(rect(&fresh, "listening-ring").is_some());
    assert!(
        rect(&fresh, "listening-label").is_some(),
        "Listening… at first"
    );
    assert!(inside(rect(&fresh, "listening").unwrap(), listening::SIZE));
    let mut later = Listening::start(now, 8.0);
    later.tick(now + std::time::Duration::from_secs(5));
    let later = runtime(later, listening::view, listening::SIZE, false);
    assert!(
        rect(&later, "listening-label").is_none(),
        "then only the glow"
    );
    assert!(rect(&later, "listening-ring").is_some());
}

#[test]
fn the_speaking_pulse_follows_the_voice_when_it_can_be_measured() {
    let dot = |level: Option<f32>| {
        let rt = runtime(
            BubbleOverlay {
                speaking: true,
                level,
                ..bubble_state(fixture::bubble_at(f64::MAX))
            },
            bubble::view,
            bubble::SIZE,
            false,
        );
        rect(&rt, "voice-dot").expect("the voice glow")[2]
    };
    assert!(dot(Some(0.9)) > dot(Some(0.02)) + 4.0, "louder grows it");
    assert_eq!(dot(None), 10.0, "unmeasured: the speaking rhythm");
    assert_eq!(bubble::pulse(0.25), 0.5);
    assert_eq!(bubble::pulse(2.0), 1.0);
}

/// The desktop while the Here question holds the pointer.
fn hands_off_desk() -> Desk {
    Desk {
        board: fixture::board_at(9.0),
        bubble: fixture::bubble_at(9.0),
        ..Desk::settled()
    }
}

#[test]
fn the_inbox_shows_one_question_with_who_asks_later_and_a_count() {
    for dark in [false, true] {
        let rt = runtime(
            orb_state(fixture::inbox_board(), false),
            orb::view,
            orb::SIZE,
            dark,
        );
        assert_eq!(count_prefix(&rt, "question-invoices"), 1, "one bubble");
        let bubble = rect(&rt, "question-invoices").expect("the active question");
        assert!(inside(bubble, orb::SIZE), "{bubble:?}");
        assert!(rect(&rt, "question-title").is_some(), "who asks");
        assert_eq!(count(&rt, "chip-needs-you"), 2, "on the card and the head");
        assert!(rect(&rt, "question-hint").is_some(), "Hold ⌘ to answer");
        assert_eq!(count(&rt, "later"), 1, "Later");
        assert_eq!(count(&rt, "questions-3"), 1, "three open questions");
    }
}

#[test]
fn the_badge_counts_every_open_question_and_none_hides_it() {
    let mut board = fixture::inbox_board();
    assert_eq!(board.open_questions(), 3);
    board.apply(
        &Event::QuestionsChanged {
            active: None,
            waiting: 0,
        },
        None,
        std::time::Instant::now(),
    );
    assert_eq!(board.open_questions(), 0);
    assert!(board.active_question().is_none());
    let rt = runtime(orb_state(board, false), orb::view, orb::SIZE, false);
    assert_eq!(count_prefix(&rt, "questions-"), 0);
    assert!(rect(&rt, "question-invoices").is_none());
}

#[test]
fn a_put_away_question_comes_back_when_another_is_active() {
    let mut board = fixture::inbox_board();
    assert!(board.dismiss_question("invoices"));
    assert!(board.active_question().is_none(), "put away");
    assert_eq!(board.open_questions(), 3, "it still waits");
    board.apply(
        &Event::QuestionsChanged {
            active: Some(crate::assistant::WaitingQuestion {
                conversation: "table".into(),
                title: "Book a table".into(),
                question: "For two or four?".into(),
                asked_ms: 2,
            }),
            waiting: 2,
        },
        None,
        std::time::Instant::now(),
    );
    let asking = board.active_question().expect("the next one");
    assert_eq!(asking.id, "table");
    assert_eq!(asking.question, "For two or four?");
}

fn state_press<S: 'static>(rt: &mut Runtime<S>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
}

static DEFERRED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[test]
fn later_puts_the_question_behind_the_others() {
    let mut state = orb_state(fixture::inbox_board(), false);
    state.actions.list.defer = |_, id| DEFERRED.lock().unwrap().push(id.to_owned());
    let mut rt = runtime(state, orb::view, orb::SIZE, false);
    state_press(&mut rt, "later");
    assert_eq!(DEFERRED.lock().unwrap().as_slice(), ["invoices"]);
}

#[test]
fn the_list_groups_waiting_running_and_done_in_that_order() {
    use stack::Group;
    let board = fixture::inbox_board();
    let groups: Vec<Group> = board.stack().iter().map(stack::Group::of).collect();
    assert!(groups.contains(&Group::Waiting));
    for dark in [false, true] {
        let rt = runtime(orb_state(board.clone(), true), orb::view, orb::SIZE, dark);
        let y = |key: &str| rect(&rt, key).unwrap_or_else(|| panic!("{key}"))[1];
        assert!(y("group-waiting") < y("group-running"));
        assert!(y("group-running") < y("group-done"));
        assert!(y("group-waiting") < y("task-invoices"));
        assert!(y("task-invoices") < y("group-running"));
        assert_eq!(count(&rt, "later"), 1, "the waiting row's Later");
    }
    // An empty group has no heading.
    let mut quiet = Board::default();
    quiet.apply(
        &Event::Started {
            conversation: "one".into(),
            origin: Origin::Background,
            question: "Sort my downloads".into(),
        },
        None,
        std::time::Instant::now(),
    );
    let rt = runtime(orb_state(quiet, true), orb::view, orb::SIZE, false);
    assert!(rect(&rt, "group-running").is_some());
    assert!(rect(&rt, "group-waiting").is_none());
    assert!(rect(&rt, "group-done").is_none());
}

#[test]
fn a_new_question_chimes_only_when_it_cant_be_read_aloud() {
    use super::host::should_chime;
    assert!(
        !should_chime(None, Some("a"), false, true),
        "it is read aloud"
    );
    assert!(
        should_chime(None, Some("a"), true, true),
        "something else plays"
    );
    assert!(
        should_chime(None, Some("a"), false, false),
        "announcements off"
    );
    assert!(
        !should_chime(Some("a"), Some("a"), true, false),
        "the same one"
    );
    assert!(!should_chime(Some("a"), None, true, false), "none left");
}

fn rest_on<S: 'static>(rt: &mut Runtime<S>, key: &str, seconds: f32) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
    rt.pointer_move(x, y);
    let mut left = seconds;
    while left > 0.0 {
        rt.frame(1.0 / 60.0);
        left -= 1.0 / 60.0;
    }
}

fn many_jobs_orb(count: usize) -> Orb {
    let mut state = orb_state(fixture::many_jobs_board(count), false);
    state.actions.list.toggle = |orb| orb.expanded = !orb.expanded;
    state
}

#[test]
fn many_jobs_show_three_calm_rows_and_one_line_for_the_rest() {
    for dark in [false, true] {
        let mut rt = runtime(many_jobs_orb(12), orb::view, orb::SIZE, dark);
        assert_eq!(count_prefix(&rt, "job-job"), 3, "three rows, no more");
        assert_eq!(count(&rt, "jobs-line"), 1, "the rest in one line");
        assert_eq!(
            text_of(&rt, "orb-count").as_deref(),
            Some("12 jobs"),
            "the head stays small: a count in words"
        );
        for key in ["orb", "jobs", "jobs-line"] {
            let r = rect(&rt, key).unwrap_or_else(|| panic!("{key}"));
            assert!(inside(r, orb::SIZE), "{key} {r:?}");
        }
        // The newest first, each with its step, in the order they started.
        let y = |rt: &Runtime<Orb>, key: &str| rect(rt, key).unwrap()[1];
        assert!(y(&rt, "job-job11") < y(&rt, "job-job10"));
        assert!(y(&rt, "job-job10") < y(&rt, "job-job9"));
        assert_eq!(count(&rt, "job-step"), 3);
        // Resting on the rows changes nothing: no list pops over them.
        rest_on(&mut rt, "job-job10", 1.0);
        rt.settle_all();
        rt.frame(0.0);
        assert!(rect(&rt, "sheet").is_none());
        assert_eq!(count_prefix(&rt, "job-job"), 3, "nothing jumps");
        // The line opens the list, where every job is.
        state_press(&mut rt, "jobs-line-text");
        rt.settle_all();
        rt.frame(0.0);
        let sheet = rect(&rt, "sheet").expect("the list");
        assert!(inside(sheet, orb::SIZE), "{sheet:?}");
        assert_eq!(count_prefix(&rt, "task-job"), 12, "every job, scrolling");
        assert_eq!(count_prefix(&rt, "job-job"), 0, "the rows make way");
    }
}

/// `docs/ux/redesign.md` §3.6: the rail holds its head, at most one
/// question, three job rows (and one "N more" line), and one result.
#[test]
fn rail_shows_at_most_three_rows_and_one_question() {
    for question in [false, true] {
        let mut board = fixture::crowd_board(12, 0);
        if question {
            fixture::crowd_question(&mut board);
        }
        for (_, kind) in [(0, false), (1, true)] {
            // With and without results to show.
            let mut board = board.clone();
            if kind {
                let landing = fixture::results_board();
                board.pops = landing.pops.clone();
                board.recent = landing.recent.clone();
            }
            let rt = runtime(orb_state(board, false), orb::view, orb::SIZE, false);
            let what = format!("question {question}, results {kind}");
            assert!(count_prefix(&rt, "job-crowd") <= 3, "{what}");
            assert_eq!(count(&rt, "jobs-line"), 1, "the rest in one line: {what}");
            assert_eq!(count(&rt, "question-text"), usize::from(question), "{what}");
            assert!(count(&rt, "result-head") <= 1, "{what}");
            assert_eq!(count(&rt, "orb"), 1, "{what}");
        }
    }
}

#[test]
fn a_press_on_a_job_s_row_opens_the_list_and_its_close_folds_it() {
    let mut rt = runtime(many_jobs_orb(2), orb::view, orb::SIZE, false);
    assert_eq!(count_prefix(&rt, "job-job"), 2, "two jobs, two rows");
    assert_eq!(count(&rt, "jobs-line"), 0, "nothing more to sum up");
    state_press(&mut rt, "job-job1");
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "sheet").is_some());
    state_press(&mut rt, "fold");
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "sheet").is_none(), "× folds it");
    assert_eq!(count_prefix(&rt, "job-job"), 2);
}

#[test]
fn escape_puts_the_field_away_then_folds_a_result_then_the_list() {
    use super::host::escape;
    let mut orb = results_orb(fixture::results_board());
    orb.reply = Some(super::ask::Reply::new("invoices"));
    orb.open_result = orb.board.pops.items.first().map(|pop| pop.key);
    orb.expanded = true;
    assert!(escape(&mut orb));
    assert!(orb.reply.is_none(), "the answer field first");
    assert!(orb.open_result.is_some() && orb.expanded);
    assert!(escape(&mut orb));
    assert!(orb.open_result.is_none(), "then the unfolded result");
    assert!(escape(&mut orb));
    assert!(!orb.expanded, "then the list");
    assert!(!escape(&mut orb), "nothing left: the key goes on");
}

#[test]
fn a_usage_limit_on_every_job_is_one_bubble() {
    let mut board = fixture::many_jobs_board(6);
    for n in 0..6 {
        board.apply(
            &Event::Waiting {
                conversation: format!("job{n}"),
                until_ms: 4_000_000_000_000 + n * 60_000,
                reason: "Claude's usage limit".into(),
            },
            None,
            std::time::Instant::now(),
        );
    }
    let rt = runtime(orb_state(board, false), orb::view, orb::SIZE, false);
    assert_eq!(count(&rt, "standing-waiting"), 1);
    assert_eq!(count_prefix(&rt, "standing-job"), 0, "no bubble each");
    for key in ["orb", "standing-waiting"] {
        let r = rect(&rt, key).unwrap();
        assert!(inside(r, orb::SIZE), "{key} {r:?}");
    }
}

#[test]
fn helpers_fold_into_their_lead_s_row() {
    let mut state = many_jobs_orb(0);
    state.board = fixture::team_board(5);
    let mut rt = runtime(state, orb::view, orb::SIZE, false);
    assert_eq!(count(&rt, "job-lead"), 1, "the lead's row");
    assert_eq!(count(&rt, "job-team"), 1, "with its helpers summed up");
    assert_eq!(count_prefix(&rt, "job-helper"), 0, "folded in");
    assert_eq!(count(&rt, "jobs-line"), 0, "helpers aren't more jobs");
    rest_on(&mut rt, "job-lead", 0.6);
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(count_prefix(&rt, "job-helper"), 0, "resting doesn't unfold");
    state_press(&mut rt, "job-lead");
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(count_prefix(&rt, "task-helper"), 5, "the list shows each");
}

static SENT: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn replying_orb() -> Orb {
    let mut state = orb_state(fixture::inbox_board(), false);
    let reply = &mut state.actions.list.reply;
    reply.start = |orb, id| orb.reply = Some(super::ask::Reply::new(id));
    reply.cancel = |orb| orb.reply = None;
    reply.send = |orb| {
        if let Some(text) = orb.reply.as_ref().and_then(super::ask::Reply::answer) {
            SENT.lock().unwrap().push(text);
            orb.reply = None;
        }
    };
    reply.edit = |orb| {
        &mut orb
            .reply
            .get_or_insert_with(super::ask::Reply::default)
            .edit
    };
    state
}

#[test]
fn a_question_is_answered_by_typing_right_at_the_orb() {
    for dark in [false, true] {
        let mut rt = runtime(replying_orb(), orb::view, orb::SIZE, dark);
        assert!(rect(&rt, "question-hint").is_some(), "Hold ⌘ to answer");
        assert!(rect(&rt, "reply-input").is_none(), "no field until asked");
        state_press(&mut rt, "reply");
        rt.settle_all();
        rt.frame(0.0);
        let field = rect(&rt, "reply-input").expect("the answer field");
        let bubble = rect(&rt, "question-invoices").unwrap();
        assert!(inside(bubble, orb::SIZE), "{bubble:?}");
        assert!(field[1] > bubble[1] && field[1] < bubble[1] + bubble[3]);
        assert!(
            rect(&rt, "question-hint").is_none(),
            "the field in its place"
        );
        assert_eq!(count(&rt, "later"), 0);
        // Nothing beside it overlaps: the rows sit past the taller bubble.
        let rows = rect(&rt, "jobs").expect("the job rows");
        assert!(rows[1] >= bubble[1] + bubble[3], "{rows:?} past {bubble:?}");
        assert!(inside(rows, orb::SIZE), "{rows:?}");
    }
    let mut rt = runtime(replying_orb(), orb::view, orb::SIZE, false);
    state_press(&mut rt, "reply");
    rt.ime(rune::Ime::Commit("Finance".into()));
    rt.frame(1.0 / 60.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Enter,
        mods: rune::Mods::default(),
    });
    for _ in 0..5 {
        rt.frame(1.0 / 60.0);
    }
    assert!(
        SENT.lock().unwrap().iter().any(|sent| sent == "Finance"),
        "Return sends it: {:?}",
        SENT.lock().unwrap()
    );
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "reply-input").is_none(), "and the field goes");
}

#[test]
fn escape_puts_the_answer_field_away() {
    let mut rt = runtime(replying_orb(), orb::view, orb::SIZE, false);
    state_press(&mut rt, "reply");
    assert!(rect(&rt, "reply-input").is_some());
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: rune::Mods::default(),
    });
    for _ in 0..5 {
        rt.frame(1.0 / 60.0);
    }
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "reply-input").is_none());
    assert!(rect(&rt, "question-hint").is_some(), "back to Hold ⌘");
}

static OPENED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static RETRIED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn results_orb(board: Board) -> Orb {
    let mut state = orb_state(board, false);
    let results = &mut state.actions.list.results;
    results.open = |_, target| OPENED.lock().unwrap().push(target.to_owned());
    results.retry = |_, id| RETRIED.lock().unwrap().push(id.to_owned());
    results.expand = |orb, key| {
        orb.open_result = (orb.open_result != Some(key)).then_some(key);
    };
    results.dismiss = |orb, key| {
        orb.board.pops.dismiss(key);
    };
    results.recall = |orb, id| {
        orb.open_result = orb.board.recall(id, std::time::Instant::now());
        orb.expanded = false;
    };
    state.actions.list.toggle = |orb| orb.expanded = !orb.expanded;
    state
}

#[test]
fn a_finished_job_leaves_a_result_with_what_to_open_and_a_failure_its_retry() {
    for dark in [false, true] {
        // The rail shows one result: the failure first, with Try again.
        let rt = runtime(
            results_orb(fixture::results_board()),
            orb::view,
            orb::SIZE,
            dark,
        );
        assert_eq!(count(&rt, "result-head"), 1, "one result in the rail");
        assert_eq!(count(&rt, "failed"), 1, "the failure first");
        assert_eq!(count(&rt, "result-retry"), 1, "Try again on the failure");
        assert!(
            count(&rt, "orb-recent") == 1,
            "nothing live: what it recalls"
        );
        for key in ["orb", "result-head"] {
            let r = rect(&rt, key).unwrap();
            assert!(inside(r, orb::SIZE), "{key} {r:?}");
        }
        // The done one: its first two targets as its actions.
        let rt = runtime(done_results_orb(), orb::view, orb::SIZE, dark);
        assert_eq!(count(&rt, "done"), 1);
        assert_eq!(
            count_prefix(&rt, "result-open-"),
            2,
            "its file and the shop; the folder when unfolded"
        );
    }
    let mut rt = runtime(
        results_orb(fixture::results_board()),
        orb::view,
        orb::SIZE,
        false,
    );
    state_press(&mut rt, "result-retry");
    assert_eq!(RETRIED.lock().unwrap().as_slice(), ["landing"]);
    let mut rt = runtime(done_results_orb(), orb::view, orb::SIZE, false);
    state_press(&mut rt, "result-more");
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(
        count_prefix(&rt, "result-open-"),
        3,
        "every target as a row"
    );
    state_press(&mut rt, "result-open-2");
    assert!(
        OPENED
            .lock()
            .unwrap()
            .iter()
            .any(|opened| opened == "/tmp/Agent-Desk Jobs/Lieferanten"),
        "{:?}",
        OPENED.lock().unwrap()
    );
}

/// The results board with only the done result up.
fn done_results_orb() -> Orb {
    let mut state = results_orb(fixture::results_board());
    state
        .board
        .pops
        .items
        .retain(|pop| pop.kind != super::pops::PopKind::Failed);
    state
}

#[test]
fn more_shows_the_whole_answer_inside_the_panel() {
    let mut rt = runtime(done_results_orb(), orb::view, orb::SIZE, false);
    assert!(rect(&rt, "result-answer").is_none(), "folded: two lines");
    state_press(&mut rt, "result-more");
    rt.settle_all();
    rt.frame(0.0);
    let answer = rect(&rt, "result-answer").expect("the whole answer");
    assert!(inside(answer, orb::SIZE), "{answer:?}");
    assert_eq!(count(&rt, "result-head"), 1, "the unfolded one alone");
}

#[test]
fn the_orb_recalls_results_after_their_bubbles_left() {
    let mut rt = runtime(
        results_orb(fixture::recall_board()),
        orb::view,
        orb::SIZE,
        false,
    );
    assert!(rect(&rt, "orb").is_some(), "it stays, quietly");
    assert_eq!(count(&rt, "result-head"), 0);
    state_press(&mut rt, "orb");
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "group-done").is_some());
    assert_eq!(count_prefix(&rt, "result-row-"), 2);
    // On its title, clear of its buttons.
    let [x, y, ..] = rect(&rt, "result-row-suppliers").unwrap();
    rt.pointer_move(x + 40.0, y + 12.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(x + 40.0, y + 12.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x + 40.0, y + 12.0);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    rt.settle_all();
    rt.frame(0.0);
    assert!(rect(&rt, "sheet").is_none(), "the list folds");
    assert!(
        rect(&rt, "result-answer").is_some(),
        "its bubble is back, unfolded"
    );
}

static RETRIED_QUICK: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// A failed quick question says why in the language it was asked in, with
/// its one action: Fix for what settings or the setup repair, Try again
/// (the same words, in full) for the rest.
#[test]
fn a_failed_quick_question_says_why_and_offers_one_action() {
    let text_of = |rt: &Runtime<BubbleOverlay>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
            .and_then(|e| e.text)
    };
    let signed_out = fixture::bubble_ended(
        "What's on my calendar today?",
        None,
        Some("Claude Code isn't signed in. Open Terminal and run “claude” to sign in."),
    );
    let mut rt = runtime(
        bubble_state(Some(signed_out)),
        bubble::view,
        bubble::SIZE,
        true,
    );
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(
        text_of(&rt, "error").as_deref(),
        Some("Your agent isn't signed in.")
    );
    assert!(text_of(&rt, "error-detail").is_some_and(|d| d.contains("Claude Code")));
    assert_eq!(count(&rt, "failure-fix"), 1);
    assert_eq!(count(&rt, "failure-retry"), 0);
    assert_eq!(count(&rt, "open-chats"), 0, "one action");

    let question = "Wie viele Termine habe ich heute noch, und wann ist der nächste?";
    let crashed = fixture::bubble_ended(question, None, Some("The task stopped unexpectedly."));
    let mut state = bubble_state(Some(crashed));
    state.actions.retry = |_, id, asked| {
        RETRIED_QUICK
            .lock()
            .unwrap()
            .push((id.to_owned(), asked.to_owned()));
    };
    let mut rt = runtime(state, bubble::view, bubble::SIZE, true);
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(
        text_of(&rt, "error").as_deref(),
        Some("Das hat nicht geklappt."),
        "in the language it was asked in"
    );
    state_press(&mut rt, "failure-retry");
    assert_eq!(
        RETRIED_QUICK
            .lock()
            .unwrap()
            .last()
            .map(|(_, asked)| asked.as_str()),
        Some(question),
        "the whole question again"
    );
}

/// Renders the overlays to `AGENT_DESK_ASSISTANT_SNAPS/assistant-*.png` in both
/// appearances, to look at: the pretend desktop in each state, the orb, the
/// list, the bubble in each state, the hints, and the Hands off pill.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
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
                "desktop-listening",
                Desk {
                    listening: Some(super::listening::Listening::start(
                        std::time::Instant::now(),
                        8.0,
                    )),
                    bubble: None,
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
        for (name, open) in [("orb-team", false), ("orb-team-open", true)] {
            let mut state = many_jobs_orb(0);
            state.board = fixture::team_board(5);
            state.expanded = open;
            let mut rt = runtime(state, orb::view, orb::SIZE, dark);
            rt.settle_all();
            let (w, h) = px(orb::SIZE);
            save(
                name,
                orb::SIZE,
                crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h),
            );
        }
        for (name, open) in [("orb-many-jobs", false), ("orb-many-jobs-open", true)] {
            let mut state = many_jobs_orb(14);
            state.expanded = open;
            let mut rt = runtime(state, orb::view, orb::SIZE, dark);
            rt.settle_all();
            let (w, h) = px(orb::SIZE);
            save(
                name,
                orb::SIZE,
                crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h),
            );
        }
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
            // The rail on the one anatomy (`docs/ux/redesign.md` §8, A).
            ("rail", orb_state(fixture::board(), false)),
            ("rail-crowd", {
                let mut board = fixture::crowd_board(6, 3);
                fixture::crowd_question(&mut board);
                orb_state(board, false)
            }),
            ("question", orb_state(fixture::inbox_board(), false)),
            ("result-failed", results_orb(fixture::results_board())),
            ("result-done", done_results_orb()),
            ("result-open", {
                let mut state = done_results_orb();
                state.open_result = state.board.pops.items.first().map(|pop| pop.key);
                state
            }),
            ("list", orb_state(fixture::board(), true)),
            ("list-recall", orb_state(fixture::recall_board(), true)),
            ("orb-inbox", orb_state(fixture::inbox_board(), false)),
            ("orb-inbox-list", orb_state(fixture::inbox_board(), true)),
            ("orb-bubbles", orb_state(fixture::board(), false)),
            ("orb-quiet", orb_state(fixture::quiet_board(), false)),
            ("orb-list", orb_state(fixture::board(), true)),
            (
                "orb-escalated",
                orb_state(fixture::escalated_board(), false),
            ),
            ("orb-results", results_orb(fixture::results_board())),
            ("orb-recall", orb_state(fixture::recall_board(), false)),
            ("orb-recall-list", orb_state(fixture::recall_board(), true)),
            ("orb-replying", {
                let mut state = replying_orb();
                let mut reply = super::ask::Reply::new("invoices");
                reply.edit.text = "Finance, please".into();
                state.reply = Some(reply);
                state
            }),
            ("orb-result-open", {
                let mut state = results_orb(fixture::results_board());
                state.open_result = state
                    .board
                    .pops
                    .items
                    .iter()
                    .find(|pop| pop.task == "suppliers")
                    .map(|pop| pop.key);
                state
            }),
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
        let now = std::time::Instant::now();
        let mut counting = super::listening::Listening::start(now, 8.0);
        counting.tick(now + std::time::Duration::from_secs(3));
        for (name, state) in [
            ("listening", super::listening::Listening::start(now, 8.0)),
            ("listening-counting", counting),
        ] {
            let size = super::listening::SIZE;
            let mut rt = runtime(state, super::listening::view, size, dark);
            let (w, h) = px(size);
            save(name, size, backdrop(&gpu, &mut rt, w, h, dark));
        }
        let mut hands_rt = runtime(hands::Hands::default(), hands::view, hands::SIZE, dark);
        let (w, h) = px(hands::SIZE);
        save(
            "hands-off",
            hands::SIZE,
            backdrop(&gpu, &mut hands_rt, w, h, dark),
        );
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
        // A mini in each of its liveness states.
        let mut rt = runtime((), minis_states, MINIS_SIZE, dark);
        let (w, h) = px(MINIS_SIZE);
        save("minis", MINIS_SIZE, backdrop(&gpu, &mut rt, w, h, dark));
        // A failed quick question, in English and in German (the window
        // and the question both).
        let failures = [
            (
                "en",
                "bubble-failed-signin-en",
                "What's on my calendar today?",
                "Claude Code isn't signed in. Open Terminal and run “claude” to sign in.",
            ),
            (
                "en",
                "bubble-failed-crash-en",
                "Summarize the PDF in front of me",
                "The task stopped unexpectedly.",
            ),
            (
                "de",
                "bubble-failed-signin-de",
                "Was steht heute noch in meinem Kalender?",
                "Claude Code isn't signed in. Open Terminal and run “claude” to sign in.",
            ),
            (
                "de",
                "bubble-failed-limit-de",
                "Fasse mir bitte das PDF zusammen, das gerade offen ist",
                "Claude Code hit its usage limit: Claude AI usage limit reached|1760000000",
            ),
        ];
        for (language, name, question, message) in failures {
            crate::desktop::i18n::apply(Some(language));
            let state = bubble_state(Some(fixture::bubble_ended(question, None, Some(message))));
            let mut rt = runtime(state, bubble::view, bubble::SIZE, dark);
            let (w, h) = px(bubble::SIZE);
            save(name, bubble::SIZE, backdrop(&gpu, &mut rt, w, h, dark));
        }
        crate::desktop::i18n::apply(Some("en"));
    }
}

const MINIS_SIZE: (f32, f32) = (380.0, 260.0);

/// A picture standing in for a page, for [`minis_states`].
fn stand_in() -> rune::Pixels {
    let (w, h) = (300u32, 188u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let top = y < 26;
            let (r, g, b) = if top {
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

/// The four looks of a mini: live, stalled, waiting for its first picture,
/// and without one.
fn minis_states(_: &(), cx: &mut Cx<()>) -> El {
    use super::look::Liveness;
    let p = super::super::controls::Paint::new(cx.system_dark(), cx.env().scale);
    let picture = stand_in();
    let none = rune::Pixels::new();
    let cell = |liveness, pixels: &rune::Pixels, title: &str, name: &str| {
        rune::col()
            .gap(3.0)
            .child(super::jobs::mini(
                p,
                format!("mini-{name}"),
                pixels,
                liveness,
                (&super::super::icons::GLOBE, title),
                true,
            ))
            .child(
                rune::text(name.to_owned())
                    .font(super::look::meta(p))
                    .color(p.muted()),
            )
    };
    super::look::material(
        p,
        rune::col().w(super::look::RAIL_W).padding(16.0).gap(12.0),
    )
    .child(
        rune::row()
            .gap(8.0)
            .child(cell(Liveness::Live, &picture, "Resy", "resy.com · live"))
            .child(cell(
                Liveness::Stalled(std::time::Duration::from_secs(8)),
                &picture,
                "Resy",
                "resy.com · stalled",
            )),
    )
    .child(
        rune::row()
            .gap(8.0)
            .child(cell(
                Liveness::Waiting,
                &none,
                "Nopa – Book a table",
                "nopa.com · waiting",
            ))
            .child(cell(
                Liveness::None,
                &none,
                "Supplier list",
                "light browser",
            )),
    )
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

/// The desk frame shows only while the agent has the person's controls:
/// clicks in parked windows, reading, and hidden browsers light nothing;
/// real input ("Hands off") and a window brought in front (a borrow) do,
/// for as long as they last.
#[test]
fn no_frame_for_work_in_parked_windows() {
    use crate::assistant::TaskWindow;
    let now = std::time::Instant::now();
    let mut board = Board::default();
    for (id, origin) in [
        ("job", Origin::Background),
        ("here", Origin::ActionShortcut),
    ] {
        board.apply(
            &Event::Started {
                conversation: id.into(),
                origin,
                question: "q".into(),
            },
            None,
            now,
        );
    }
    assert_eq!(board.frame_until(now), None);
    // A job clicking and reading in its parked windows, a Here question
    // reading the screen: no frame.
    for (id, looking) in [("job", false), ("job", true), ("here", true)] {
        board.apply(
            &Event::Acting {
                conversation: id.into(),
                looking,
            },
            None,
            now,
        );
        assert_eq!(board.frame_until(now), None, "{id} {looking}");
    }
    // Real input under "Hands off" holds it while it lasts.
    board.apply(
        &Event::HandsOff {
            conversation: "here".into(),
            on: true,
        },
        None,
        now,
    );
    assert!(board.frame_until(now).is_some());
    board.apply(
        &Event::HandsOff {
            conversation: "here".into(),
            on: false,
        },
        None,
        now,
    );
    assert_eq!(board.frame_until(now), None);
    // A window brought in front: the frame until it goes back.
    board.apply(
        &Event::Borrowed {
            conversation: "job".into(),
            window: TaskWindow {
                app: "Numbers".into(),
                window_id: Some("w-7".into()),
            },
            why: crate::assistant::guard::BORROW_KEYS.into(),
            until_ms: 20_000,
        },
        None,
        now,
    );
    assert!(board.frame_until(now).is_some());
    board.apply(
        &Event::Returned {
            conversation: "job".into(),
            to: Some("Mail".into()),
        },
        None,
        now,
    );
    assert_eq!(board.frame_until(now), None);
}
