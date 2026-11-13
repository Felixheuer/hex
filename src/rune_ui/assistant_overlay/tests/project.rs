//! Projects on the overlay: the chip at the rail's head, its picker, a
//! job's tag, and the Here bubble's one-tap offer; rendered (ignored, with
//! a GPU) in both appearances.

use super::*;
use crate::assistant::projects::Snapshot;
use crate::assistant::Event;
use super::super::project::Picker;

fn snapshot(active: Option<&str>) -> Snapshot {
    let recent: Vec<(String, String)> = ["Startup", "Privat", "Kalibermarkt"]
        .into_iter()
        .map(|name| (format!("id-{name}"), name.to_owned()))
        .collect();
    Snapshot {
        active: active.map(|name| (format!("id-{name}"), name.to_owned())),
        recent,
    }
}

fn idle_rail(active: Option<&str>, open: bool) -> Orb {
    let mut state = orb_state(Board::default(), false);
    state.project = Picker {
        snapshot: snapshot(active),
        open,
        naming: None,
    };
    state
}

/// Three jobs, the newest in the active project "Startup", one in "Privat".
fn busy_rail() -> Orb {
    let mut board = fixture::many_jobs_board(3);
    let now = std::time::Instant::now();
    board.apply(
        &Event::InProject {
            conversation: "job2".into(),
            name: "Startup".into(),
        },
        None,
        now,
    );
    board.apply(
        &Event::InProject {
            conversation: "job1".into(),
            name: "Privat".into(),
        },
        None,
        now,
    );
    let mut state = orb_state(board, false);
    state.project.snapshot = snapshot(Some("Startup"));
    state
}

fn offered_bubble() -> BubbleOverlay {
    let mut bubble = fixture::bubble_ended(
        "Wie viele Bestellungen hatte kalibermarkt.de heute?",
        Some("Heute kamen **37 Bestellungen** über kalibermarkt.de, 8 mehr als gestern."),
        None,
    );
    bubble.suggested = Some(("id-Kalibermarkt".into(), "Kalibermarkt".into()));
    bubble_state(Some(bubble))
}

#[test]
fn an_idle_rail_shows_only_the_active_project_s_chip() {
    for dark in [false, true] {
        let rt = runtime(idle_rail(Some("Startup"), false), orb::view, orb::SIZE, dark);
        assert_eq!(text_of(&rt, "project-chip-name").as_deref(), Some("Startup"));
        assert_eq!(count(&rt, "orb"), 0, "nothing at work: no orb, the chip alone");
        assert_eq!(count(&rt, "project-picker"), 0);
        assert!(inside(rect(&rt, "project-chip").unwrap(), orb::SIZE));
    }
    // No project and nothing at work: nothing on the screen.
    let rt = runtime(idle_rail(None, false), orb::view, orb::SIZE, false);
    assert_eq!(count(&rt, "project-chip"), 0);
    assert!(!idle_rail(None, false).project.shows());
    assert!(idle_rail(Some("Startup"), false).project.shows());
}

#[test]
fn the_picker_offers_the_recent_projects_no_project_and_a_new_one() {
    for dark in [false, true] {
        let rt = runtime(idle_rail(Some("Startup"), true), orb::view, orb::SIZE, dark);
        for key in ["project-Startup", "project-Privat", "project-Kalibermarkt", "project-none", "project-new"] {
            assert_eq!(count(&rt, key), 1, "{key}");
        }
        let picker = rect(&rt, "project-picker").unwrap();
        let chip = rect(&rt, "project-chip").unwrap();
        assert!(picker[1] > chip[1], "it unfolds under the chip");
        assert!(inside(picker, orb::SIZE));
        // The active one comes first, ticked.
        let y = |key: &str| rect(&rt, key).unwrap()[1];
        assert!(y("project-Startup") < y("project-Privat"));
        assert!(y("project-Kalibermarkt") < y("project-none"));
        assert!(y("project-none") < y("project-new"));
    }
    // Opened with no project: the chip says so, "No project" is ticked.
    let rt = runtime(idle_rail(None, true), orb::view, orb::SIZE, false);
    assert_eq!(text_of(&rt, "project-chip-name").as_deref(), Some("No project"));
    assert_eq!(count(&rt, "project-picker"), 1);
}

static PICKED: std::sync::Mutex<Vec<Option<String>>> = std::sync::Mutex::new(Vec::new());

#[test]
fn a_press_picks_a_project_and_new_project_asks_for_its_name() {
    let mut state = idle_rail(Some("Startup"), true);
    state.actions.project.pick = |orb, id| {
        PICKED.lock().unwrap().push(id.map(str::to_owned));
        orb.project.open = false;
    };
    state.actions.project.name = |orb| {
        let mut edit = rune::TextEdit::default();
        edit.focused = true;
        orb.project.naming = Some(edit);
    };
    state.actions.project.edit = |orb| orb.project.naming.get_or_insert_with(Default::default);
    let mut rt = runtime(state, orb::view, orb::SIZE, false);
    state_press(&mut rt, "project-new");
    assert_eq!(count(&rt, "project-name-input"), 1, "the name is typed in place");
    assert_eq!(count(&rt, "project-new"), 0);
    state_press(&mut rt, "project-Privat");
    assert_eq!(
        PICKED.lock().unwrap().last().cloned(),
        Some(Some("id-Privat".to_owned()))
    );
    assert_eq!(count(&rt, "project-picker"), 0, "it folds once picked");
}

#[test]
fn a_job_in_another_project_wears_a_tag_and_the_busy_head_keeps_the_chip() {
    for dark in [false, true] {
        let rt = runtime(busy_rail(), orb::view, orb::SIZE, dark);
        assert_eq!(count(&rt, "orb"), 1);
        assert_eq!(text_of(&rt, "project-chip-name").as_deref(), Some("Startup"));
        // job1 is in Privat, job2 in the active Startup, job0 in none.
        assert_eq!(count(&rt, "job-project"), 1, "only the other project's job");
        let tag = rect(&rt, "job-project").unwrap();
        let row = rect(&rt, "job-job1").unwrap();
        assert!(tag[1] >= row[1] && tag[1] < row[1] + 40.0, "on job1's head");
        for key in ["orb", "project-chip"] {
            assert!(inside(rect(&rt, key).unwrap(), orb::SIZE), "{key}");
        }
    }
}

#[test]
fn a_crowded_head_keeps_the_chip_short_and_inside_the_panel() {
    for active in [None, Some("Kalibermarkt Relaunch 2027")] {
        let mut board = fixture::crowd_board(12, 5);
        fixture::crowd_question(&mut board);
        let mut state = orb_state(board, false);
        state.desk = fixture::crowd_desk();
        state.project.snapshot = snapshot(active);
        let rt = runtime(state, orb::view, orb::SIZE, true);
        for key in ["orb", "project-chip", "orb-all-live"] {
            let r = rect(&rt, key).unwrap_or_else(|| panic!("{key}"));
            assert!(inside(r, orb::SIZE), "{key} {r:?} with {active:?}");
        }
        match active {
            Some(_) => assert_eq!(text_of(&rt, "project-chip-name").as_deref(), Some("Kalibermark…")),
            None => assert_eq!(count(&rt, "project-chip-name"), 0, "no project: the dot alone"),
        }
    }
}

#[test]
fn a_request_about_a_project_is_offered_to_it_with_one_tap() {
    for dark in [false, true] {
        let rt = runtime(offered_bubble(), bubble::view, bubble::SIZE, dark);
        assert_eq!(count(&rt, "project-offer"), 1);
        assert_eq!(count(&rt, "open-chats"), 1, "More stays");
    }
    // A project chosen for it, or none suggested: no offer.
    let mut state = offered_bubble();
    if let Some(bubble) = state.bubble.as_mut() {
        bubble.suggested = None;
    }
    let rt = runtime(state, bubble::view, bubble::SIZE, false);
    assert_eq!(count(&rt, "project-offer"), 0);
}

#[test]
fn the_offer_comes_with_its_event_and_goes_once_the_task_is_in_the_project() {
    use super::super::board::Bubble;
    let now = std::time::Instant::now();
    let mut bubble = Some(Bubble::new("q".into(), "kalibermarkt.de?"));
    assert!(Bubble::apply(
        &mut bubble,
        &Event::ProjectSuggested {
            conversation: "q".into(),
            id: "k".into(),
            name: "Kalibermarkt".into(),
        },
        now
    ));
    assert_eq!(
        bubble.as_ref().unwrap().suggested,
        Some(("k".into(), "Kalibermarkt".into()))
    );
    assert!(Bubble::apply(
        &mut bubble,
        &Event::InProject {
            conversation: "q".into(),
            name: "Kalibermarkt".into(),
        },
        now
    ));
    assert_eq!(bubble.unwrap().suggested, None);
}

#[test]
fn while_held_the_island_names_the_project_the_words_go_to() {
    use super::super::island;
    use super::super::quick::Held;
    let mut held = Held::new(Some("Mail".into()));
    let line = island::line(Some(&held), None).unwrap();
    assert_eq!(line.trail.as_deref(), Some("Mail in front"));
    held.project = Some("Startup".into());
    let line = island::line(Some(&held), None).unwrap();
    assert_eq!(line.trail.as_deref(), Some("Mail in front · ● Startup"));
    let line = island::line(Some(&Held { project: Some("Startup".into()), ..Held::default() }), None).unwrap();
    assert_eq!(line.trail.as_deref(), Some("● Startup"));
}

/// Renders the chip, the picker (with the name field), the busy rail with
/// a job's tag, and the bubble's offer to `AGENT_DESK_PROJECT_SNAPS`.
#[test]
#[ignore = "set AGENT_DESK_PROJECT_SNAPS to a folder; needs a GPU"]
fn project_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_PROJECT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    let px = |size: (f32, f32)| ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let save = |name: &str, size: (f32, f32), pixels: Vec<u8>| {
            let (w, h) = px(size);
            let path = folder.join(format!("project-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        };
        // The rail's top: a third of the panel is enough to look at.
        let rail = (orb::SIZE.0, 420.0);
        let mut naming = idle_rail(Some("Startup"), true);
        let mut edit = rune::TextEdit::new("Kalibermarkt Relaunch");
        edit.focused = true;
        naming.project.naming = Some(edit);
        naming.actions.project.edit =
            |orb| orb.project.naming.get_or_insert_with(Default::default);
        for (name, state) in [
            ("chip", idle_rail(Some("Startup"), false)),
            ("picker", idle_rail(Some("Startup"), true)),
            ("picker-none", idle_rail(None, true)),
            ("picker-new", naming),
        ] {
            let mut rt = runtime(state, orb::view, rail, dark);
            let (w, h) = px(rail);
            save(name, rail, crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h));
        }
        let mut rt = runtime(busy_rail(), orb::view, orb::SIZE, dark);
        let (w, h) = px(orb::SIZE);
        save("rail-jobs", orb::SIZE, crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h));
        let mut board = fixture::crowd_board(12, 5);
        fixture::crowd_question(&mut board);
        let mut crowded = orb_state(board, false);
        crowded.project.snapshot = snapshot(Some("Kalibermarkt Relaunch 2027"));
        let mut rt = runtime(crowded, orb::view, rail, dark);
        let (w, h) = px(rail);
        save("rail-crowded", rail, crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h));
        let mut rt = runtime(offered_bubble(), bubble::view, bubble::SIZE, dark);
        let (w, h) = px(bubble::SIZE);
        save("offer", bubble::SIZE, crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h));
    }
}
