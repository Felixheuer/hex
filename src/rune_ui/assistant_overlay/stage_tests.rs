//! The stage's rules, its panels laid out offscreen, and (ignored, with a
//! GPU) rendered to `assistant-stage-*-{light,dark}.png`.

use std::time::{Duration, Instant};

use rune::text::TextSystem;
use rune::{Env, Runtime};

use super::*;

/// A long answer (about 2 000 characters) whose last line must show once
/// scrolled, never cut with "…".
pub(crate) const LONG_ANSWER: &str = "**Three venues fit 14 people on Thursday:**\n\n\
1. **Kantine Kreuzberg**: private room, 14–18 people, 45 € per head, walkable from the office, menu on their site.\n\
2. **Prinzessinnengarten**: outdoor with a tent, weather risk, 38 € per head, needs a deposit of 200 €.\n\
3. **Spreeblick Loft**: projector and kitchen, 900 € flat for the evening, answers bookings within a day.\n\n\
Maya prefers a room with a screen, so *Spreeblick* is the closest match; Kantine is cheaper and the only one with a set menu.\n\n\
- Kantine has Thursday 18:00 free (checked on their site a minute ago)\n\
- Spreeblick asks for the headcount two days ahead\n\
- Prinzessinnengarten closes the tent at 22:00\n\
- All three are within 20 minutes by U-Bahn from the office\n\n\
**Getting there.** From the office, Kantine is a ten-minute walk along the canal; Spreeblick is two stops on the U1 \
and then five minutes on foot; Prinzessinnengarten is at Moritzplatz, one stop on the U8. If it rains, only Kantine and \
Spreeblick keep everyone dry, and Spreeblick lets you bring your own drinks for a small fee.\n\n\
**Budget.** At 14 people Kantine costs about 630 €, Prinzessinnengarten about 532 € plus the deposit, and Spreeblick \
900 € flat, which gets cheaper per head if more people come. All prices include tax; none includes drinks except the \
Kantine menu, which has one drink per person.\n\n\
**Food and access.** Kantine cooks on site and can do vegan and gluten-free plates if told two days ahead; Spreeblick \
has a kitchen but no cook, so it works best with a caterer (they recommend two nearby); Prinzessinnengarten has a café \
that closes at 19:00. Kantine and Spreeblick have step-free entrances and lifts; the garden has gravel paths. Parking is hard at all three; bikes fit in the yard at Kantine and the hall at Spreeblick.\n\n\
I saved the three offers in `/tmp/Agent-Desk Jobs/Offsite/Offers.pdf` so you can forward them.\n\n\
Say “book Kantine” or “ask Spreeblick” and I'll do it. LAST LINE OF THE ANSWER.";

const SUPPLIERS: &str = "Hof Brandt is cheapest (3,80 €/kg) but slowest; Agrar Süd is the best balance.\n\n\
- **Müller GmbH**, Hamburg: 4,20 €/kg, 5 days\n\
- **Nordkorn**, Kiel: 3,95 €/kg, 8 days\n\
- **Agrar Süd**, Ulm: 4,05 €/kg, 4 days\n\
- **Hof Brandt**, Celle: 3,80 €/kg, 10 days\n\n\
- Sheet: `/tmp/Agent-Desk Jobs/Flour suppliers/Suppliers.numbers`\n\
- Shared: [the team's copy](https://docs.example.com/spreadsheets/flour)\n\n\
Files: /tmp/Agent-Desk Jobs/Flour suppliers";

fn outcome(failed: bool) -> Outcome {
    Outcome {
        id: "suppliers".into(),
        title: "Compare 50 flour suppliers".into(),
        colour: 2,
        failed,
        text: if failed {
            "The site asked for a login the job doesn't have (xyz.com/admin).".into()
        } else {
            SUPPLIERS.into()
        },
        at: Instant::now(),
    }
}

fn steps() -> Vec<String> {
    [
        "Searched the web for flour suppliers near Berlin",
        "Opened 50 supplier pages in the background browser",
        "Read each price list and lead time",
        "Wrote Suppliers.numbers in the job's folder",
        "Shared the sheet with the team",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn result_panel(failed: bool) -> Panel {
    Panel::Result {
        outcome: outcome(failed),
        steps: steps(),
        log: Some("/tmp/Agent-Desk Jobs/Flour suppliers/Agent-Desk-Protokoll.txt".into()),
    }
}

fn answer_panel() -> Panel {
    Panel::Answer(Answer {
        id: "offsite".into(),
        question: "Where can we hold the offsite on Thursday?".into(),
        text: LONG_ANSWER.into(),
    })
}

fn stage(panel: Panel) -> Stage {
    let mut stage = Stage::new("⌘".into(), Actions::inert());
    stage.show(panel);
    stage
}

fn runtime(state: Stage, dark: bool) -> Runtime<Stage> {
    let mut rt = crate::rune_ui::offscreen::runtime(
        state,
        view,
        SIZE,
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

fn keyed<'a>(rt: &'a Runtime<Stage>, prefix: &'a str) -> Vec<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with(prefix)))
        .collect()
}

fn press(rt: &mut Runtime<Stage>, key: &str) {
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

#[test]
fn a_long_answer_scrolls_in_full_and_is_never_cut() {
    assert!(
        LONG_ANSWER.chars().count() >= 1_900,
        "about 2 000 characters"
    );
    let mut rt = runtime(stage(answer_panel()), false);
    let viewport = keyed(&rt, "stage-answer")[0].rect;
    assert!(viewport[3] <= ANSWER_MAX_H + 0.5, "{viewport:?}");
    let last_block = |rt: &Runtime<Stage>| {
        rt.elements()
            .into_iter()
            .filter(|e| {
                e.text
                    .as_deref()
                    .is_some_and(|t| t.contains("LAST LINE OF THE ANSWER"))
            })
            .map(|e| e.rect)
            .next()
            .expect("the whole answer, to its last line, is on the stage")
    };
    let end = last_block(&rt);
    assert!(
        end[1] + end[3] > viewport[1] + viewport[3],
        "the end is past the fold: {end:?} {viewport:?}"
    );
    // Scrolled with the wheel, the end comes into view.
    let (x, y) = (
        viewport[0] + viewport[2] / 2.0,
        viewport[1] + viewport[3] / 2.0,
    );
    rt.pointer_move(x, y);
    for _ in 0..60 {
        rt.wheel(x, y, -120.0);
        rt.frame(1.0 / 60.0);
    }
    rt.settle_all();
    rt.frame(0.0);
    let scrolled = last_block(&rt);
    assert!(
        scrolled[1] < end[1],
        "it scrolled: {scrolled:?} from {end:?}"
    );
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter_map(|e| e.text.or(e.label))
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("LAST LINE OF THE ANSWER")),
        "the whole answer is on the stage: {texts:?}"
    );
    assert!(
        !texts
            .iter()
            .any(|t| t.ends_with('…') && t.contains("Kantine")),
        "nothing is cut with …"
    );
    let card = keyed(&rt, "stage-card")[0].rect;
    assert!(
        card[1] + card[3] <= SIZE.1,
        "the card fits the panel: {card:?}"
    );
}

#[test]
fn every_target_row_has_a_verb() {
    for panel in [answer_panel(), result_panel(false)] {
        let targets = panel.targets();
        assert!(!targets.is_empty());
        let rt = runtime(stage(panel), true);
        for index in 0..targets.len() {
            assert!(
                !keyed(&rt, &format!("stage-verb-{index}-")).is_empty(),
                "row {index} ({:?}) has a verb",
                targets[index]
            );
        }
        assert_eq!(keyed(&rt, "stage-primary").len(), 1, "one primary");
    }
}

#[test]
fn a_result_names_its_folder_file_and_link_with_their_verbs() {
    let targets = result_panel(false).targets();
    let kinds: Vec<Kind> = targets.iter().map(|t| t.kind).collect();
    assert!(
        kinds.contains(&Kind::File) && kinds.contains(&Kind::Link) && kinds.contains(&Kind::Folder)
    );
    assert_eq!(verbs(Kind::File), [Verb::Show, Verb::Open]);
    assert_eq!(verbs(Kind::Folder), [Verb::OpenFolder]);
    assert_eq!(verbs(Kind::Link), [Verb::OpenLink]);
    // The sheet is what the primary opens.
    let main = main_target(&targets).unwrap();
    assert_eq!(targets[main].name, "Suppliers.numbers");
    let home = std::path::Path::new("/Users/maya");
    let file = Target {
        kind: Kind::File,
        open: "/Users/maya/Agent-Desk Jobs/Offers.pdf".into(),
        name: "Offers.pdf".into(),
    };
    assert_eq!(place(&file, Some(home)), "~/Agent-Desk Jobs");
}

#[test]
fn spoken_verbs_act_on_the_panels_targets() {
    let targets = result_panel(false).targets();
    let at = |kind: Kind| targets.iter().position(|t| t.kind == kind).unwrap();
    let cases = [
        (
            "open the folder",
            Some((at(Kind::Folder), Verb::OpenFolder)),
        ),
        (
            "Show the folder.",
            Some((at(Kind::Folder), Verb::OpenFolder)),
        ),
        ("open the sheet", Some((at(Kind::File), Verb::Open))),
        ("show me the file", Some((at(Kind::File), Verb::Show))),
        (
            "Öffne den Ordner",
            Some((at(Kind::Folder), Verb::OpenFolder)),
        ),
        ("zeig mir die Datei", Some((at(Kind::File), Verb::Show))),
        ("open the link", Some((at(Kind::Link), Verb::OpenLink))),
        ("open it", Some((at(Kind::File), Verb::Open))),
        ("open suppliers", Some((at(Kind::File), Verb::Open))),
        ("open Safari and book a table for two at eight", None),
        ("what's the cheapest one?", None),
        ("open", None),
    ];
    for (words, expected) in cases {
        assert_eq!(spoken(words, &targets), expected, "{words}");
    }
    assert_eq!(spoken("open the folder", &[]), None, "nothing to open");
}

#[test]
fn what_it_did_shows_the_steps_and_back_returns() {
    let mut rt = runtime(stage(result_panel(false)), false);
    press(&mut rt, "stage-steps");
    assert!(matches!(rt.state.panel, Some(Panel::Steps(_))));
    assert_eq!(
        keyed(&rt, "stage-step-").len() - keyed(&rt, "stage-step-list").len(),
        5
    );
    press(&mut rt, "stage-back");
    assert!(matches!(rt.state.panel, Some(Panel::Result { .. })));
}

#[test]
fn the_window_is_only_behind_the_overflow_menu() {
    static ASKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut state = stage(answer_panel());
    state.actions.window = |_, id| ASKED.lock().unwrap().push(id.to_owned());
    let mut rt = runtime(state, false);
    assert!(
        keyed(&rt, "stage-open-window").is_empty(),
        "not in plain sight"
    );
    press(&mut rt, "stage-menu-button");
    press(&mut rt, "stage-open-window");
    assert_eq!(ASKED.lock().unwrap().as_slice(), ["offsite"]);
}

#[test]
fn pinning_keeps_a_panel_and_a_new_one_unpins() {
    let mut rt = runtime(stage(answer_panel()), false);
    press(&mut rt, "stage-pin");
    assert!(rt.state.pinned);
    let now = Instant::now();
    assert_eq!(folds_at(now, true, false), None);
    assert_eq!(folds_at(now, false, true), None, "pointed at");
    assert_eq!(folds_at(now, false, false), Some(now + STAYS));
    rt.state.show(result_panel(false));
    assert!(!rt.state.pinned);
    let _ = Duration::ZERO;
}

#[test]
fn a_failed_result_offers_try_again_and_no_targets() {
    let rt = runtime(stage(result_panel(true)), false);
    assert_eq!(keyed(&rt, "stage-retry").len(), 1);
    assert!(keyed(&rt, "stage-target-").is_empty());
    assert!(keyed(&rt, "stage-primary").is_empty());
}

#[test]
fn the_repair_card_fixes_in_place_and_keeps_the_guide_deliberate() {
    assert_eq!(in_place(&Gap::Accessibility), Some(InPlace::Accessibility));
    assert_eq!(in_place(&Gap::AgentSignIn), Some(InPlace::SignIn));
    assert_eq!(in_place(&Gap::Shortcut), None);
    let rt = runtime(
        stage(Panel::Repair(vec![Gap::Accessibility, Gap::Shortcut])),
        false,
    );
    assert_eq!(
        keyed(&rt, "stage-fix").len() - keyed(&rt, "stage-fix-all").len(),
        1
    );
    assert_eq!(keyed(&rt, "stage-fix-all").len(), 1);
    assert!(
        !Panel::Repair(Vec::new()).takes_keys(),
        "it never takes the keyboard"
    );
    let rt = runtime(stage(Panel::Repair(vec![Gap::Shortcut])), false);
    assert_eq!(
        keyed(&rt, "stage-fix-all").len(),
        1,
        "only the guide fixes it"
    );
}

#[test]
fn notes_have_a_safe_name_and_the_question_as_heading() {
    assert_eq!(
        note_name("Where can we hold the offsite?"),
        "Where can we hold the offsite.md"
    );
    assert_eq!(note_name("a/b:c"), "a b c.md");
    assert_eq!(note_name("   "), "Agent-Desk note.md");
    assert!(note_name(&"x".repeat(200)).chars().count() <= 63);
    let (title, text) = note_text(&answer_panel()).unwrap();
    assert_eq!(title, "Where can we hold the offsite on Thursday?");
    assert!(text.starts_with("# Where can we hold"));
    assert!(text.contains("LAST LINE"));
}

#[test]
fn the_rest_of_a_result_leaves_out_its_headline_and_folder_line() {
    let rest = rest_of(SUPPLIERS);
    assert!(!rest.contains("Hof Brandt is cheapest"));
    assert!(!rest.contains("Files:"));
    assert!(rest.contains("Nordkorn"));
}

/// Renders the stage to `AGENT_DESK_ASSISTANT_SNAPS/assistant-stage-*.png`
/// in both appearances.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn stage_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let pinned = {
            let mut state = stage(answer_panel());
            state.pinned = true;
            state.copied = true;
            state
        };
        let menu = {
            let mut state = stage(result_panel(false));
            state.menu = true;
            state
        };
        let steps = {
            let mut state = stage(result_panel(false));
            state.what_it_did();
            state
        };
        let panels = [
            ("answer", stage(answer_panel())),
            ("results", stage(result_panel(false))),
            ("pinned", pinned),
            ("menu", menu),
            ("steps", steps),
            ("failed", stage(result_panel(true))),
            (
                "repair",
                stage(Panel::Repair(vec![Gap::Accessibility, Gap::AgentSignIn])),
            ),
        ];
        for (name, state) in panels {
            let mut rt = runtime(state, dark);
            let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
            let pixels = backdrop(&gpu, &mut rt, w, h, dark);
            let path = folder.join(format!("assistant-stage-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        }
    }
}

/// `rt` drawn over a desktop grey, as the clear overlay sits on screen.
fn backdrop(
    gpu: &rune::render::Gpu,
    rt: &mut Runtime<Stage>,
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
