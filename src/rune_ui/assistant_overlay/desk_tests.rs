//! The whole desk, every overlay at once, in the combinations the person
//! meets (`docs/ux/redesign.md` §8, integration pass): the placement rules
//! of §3.1 hold in each (laid out offscreen), and (ignored, with a GPU)
//! each renders to `assistant-desk-*-{light,dark}.png` to look at.

use std::time::Instant;

use rune::text::TextSystem;
use rune::{Env, Runtime};

use super::super::fixture;
use super::super::frame::Tab;
use super::super::quick::Held;
use super::super::results::Outcome;
use super::super::stage::{Panel, Stage};
use super::*;

fn runtime(desk: Desk, dark: bool) -> Runtime<Desk> {
    let size = desk.size;
    let mut rt = crate::rune_ui::offscreen::runtime(
        desk,
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

fn rects(rt: &Runtime<Desk>, key: &str) -> Vec<[f32; 4]> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some(key))
        .map(|e| e.rect)
        .collect()
}

fn overlap([ax, ay, aw, ah]: [f32; 4], [bx, by, bw, bh]: [f32; 4]) -> f32 {
    let w = (ax + aw).min(bx + bw) - ax.max(bx);
    let h = (ay + ah).min(by + bh) - ay.max(by);
    w.max(0.0) * h.max(0.0)
}

/// A finished job's result, for the stage.
fn result_panel() -> Panel {
    Panel::Result {
        outcome: Outcome {
            id: "suppliers".into(),
            title: "Compare 50 flour suppliers".into(),
            colour: 2,
            failed: false,
            text: "Agrar Süd is the best balance of price and speed.\n\n\
                   - **Müller GmbH**, Hamburg: 4,20 €/kg, 5 days\n\
                   - **Agrar Süd**, Ulm: 4,05 €/kg, 4 days\n\n\
                   Files: /tmp/Agent-Desk Jobs/Flour suppliers"
                .into(),
            at: Instant::now(),
            project: None,
        },
        steps: vec![
            "Read each price list".into(),
            "Wrote Suppliers.numbers".into(),
        ],
        log: None,
    }
}

fn staged() -> Stage {
    let mut stage = Stage::new(
        fixture::ANSWER_KEY.into(),
        super::super::stage::Actions::inert(),
    );
    stage.show(result_panel());
    stage
}

fn borrow() -> Tab {
    Tab::InFront {
        app: "Mail".into(),
        window: fixture::window("mail"),
        why: crate::assistant::guard::BORROW_KEYS.into(),
        until_ms: super::super::when::now_ms() + 12_000,
    }
}

fn held() -> Held {
    let mut held = Held::new(Some("Mail".into()));
    held.hear(
        "Reply to Maya that Thursday works and book Nopa for two at",
        "",
    );
    held
}

/// Every combination looked at: (name, desk).
fn desks() -> Vec<(&'static str, Desk)> {
    let big = (1800.0, 1169.0);
    let mut question = fixture::crowd_board(6, 3);
    fixture::crowd_question(&mut question);
    vec![
        ("story", Desk::settled()),
        (
            "one-job",
            Desk {
                board: fixture::quiet_board(),
                ..Desk::settled()
            },
        ),
        (
            "many-jobs",
            Desk {
                board: fixture::many_jobs_board(9),
                bubble: fixture::bubble_at(9.1),
                ..Desk::settled()
            },
        ),
        (
            "question",
            Desk {
                board: question,
                bubble: None,
                ..Desk::settled()
            },
        ),
        (
            "stage",
            Desk {
                stage: Some(staged()),
                bubble: None,
                ..Desk::settled()
            },
        ),
        (
            "borrow",
            Desk {
                tab: Some(borrow()),
                bubble: fixture::bubble_at(9.1),
                ..Desk::settled()
            },
        ),
        ("hands-off", super::super::tests::hands_off_desk()),
        (
            "held",
            Desk {
                held: Some(held()),
                bubble: None,
                ..Desk::settled()
            },
        ),
        (
            "flat",
            Desk {
                notch: false,
                ..Desk::settled()
            },
        ),
        (
            "big",
            Desk {
                size: big,
                ..Desk::settled()
            },
        ),
        (
            "big-flat-borrow",
            Desk {
                size: big,
                notch: false,
                tab: Some(borrow()),
                bubble: fixture::bubble_at(9.1),
                ..Desk::settled()
            },
        ),
    ]
}

/// §3.1: the island's card, the rail, the tab, and the stage never cover
/// an outlined window's label (as drawn, shortened beside them where it
/// had to) or the focus window's title bar; the island hangs from the
/// notch, or under the menu bar's centre without one; the tab hangs under
/// the island and the card under both.
#[test]
fn nothing_covers_a_label_or_the_focus_title_bar_on_any_desk() {
    for language in ["de", "en"] {
        crate::desktop::i18n::apply(Some(language));
        covers_nothing();
    }
}

fn covers_nothing() {
    for (name, desk) in desks() {
        let notch = desk.notch;
        let (w, _) = desk.size;
        let line = island::line_with(
            desk.held.as_ref(),
            desk.tab.as_ref(),
            desk.bubble.as_ref().filter(|_| !desk.setup),
        );
        let at = layout(&desk, line.as_ref());
        let rt = runtime(desk, false);
        if let Some([ix, iy, iw, _]) = rects(&rt, "island").first().copied() {
            assert!((ix + iw / 2.0 - w / 2.0).abs() < 1.0, "{name}: centred");
            if notch {
                assert!(iy.abs() < 0.5, "{name}: grows from the notch, {iy}");
            } else {
                assert!(
                    iy >= MENU_BAR,
                    "{name}: a free capsule under the menu bar, {iy}"
                );
            }
        }
        let mut cards: Vec<[f32; 4]> = Vec::new();
        cards.extend(rects(&rt, "cluster"));
        cards.extend(rects(&rt, "frame-tab"));
        cards.extend(rects(&rt, "frame-back"));
        for bubble in rects(&rt, "bubble") {
            let inset = bubble::INSET;
            let [x, y, bw, bh] = bubble;
            cards.push([x + inset, y + inset, bw - 2.0 * inset, bh - 2.0 * inset]);
        }
        // The outlines' chips (other parts keyed "label" are smaller).
        let labels = rects(&rt, "label")
            .into_iter()
            .filter(|label| (label[3] - outline::LABEL_H).abs() < 0.5);
        for label in labels {
            for card in &cards {
                assert!(
                    overlap(label, *card) < 1.0,
                    "{name}: {card:?} covers the label at {label:?}"
                );
            }
        }
        if let Some(bar) = at.focus {
            for card in &cards {
                assert!(
                    overlap(bar, *card) < 1.0,
                    "{name}: {card:?} covers the focus window's title bar"
                );
            }
        }
        if let (Some(tab), Some(card)) = (at.tab, at.card) {
            assert!(
                card[1] >= tab[1] + tab[3],
                "{name}: the card hangs under the tab"
            );
        }
        if let Some(card) = at.card {
            assert!(
                card[1] >= at.island[1] + at.island[3],
                "{name}: the card hangs under the island"
            );
        }
    }
}

/// §3.6: at most five floating cards: the island's, the stage's, and the
/// rail's share of the rest.
#[test]
fn at_most_five_floating_cards_on_any_desk() {
    for (name, desk) in desks() {
        let line = island::line_with(desk.held.as_ref(), desk.tab.as_ref(), desk.bubble.as_ref());
        let at = layout(&desk, line.as_ref());
        let island = usize::from(at.card.is_some());
        let stage = usize::from(at.stage.is_some());
        assert_eq!(
            at.rail_cards + island + stage,
            orb::MAX_CARDS,
            "{name}: the rail gets what's left"
        );
        // The rail shows no more than its share.
        let board = &desk.board;
        let running = super::super::jobs::running(&board.tasks);
        let question = usize::from(board.active_question().is_some());
        let left = at.rail_cards.saturating_sub(question);
        let rows = super::super::jobs::layout_at_most(&running, question > 0, left)
            .rows
            .len();
        assert!(
            question + rows <= at.rail_cards,
            "{name}: {question} + {rows} rows within {}",
            at.rail_cards
        );
    }
}

/// Renders every combination to `AGENT_DESK_ASSISTANT_SNAPS/assistant-desk-*`.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn desk_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    for language in ["en", "de"] {
        crate::desktop::i18n::apply(Some(language));
        for dark in [false, true] {
            let theme = if dark { "dark" } else { "light" };
            for (name, desk) in desks() {
                if language == "de" && !matches!(name, "story" | "borrow" | "held" | "stage") {
                    continue;
                }
                let (w, h) = ((desk.size.0 * 2.0) as u32, (desk.size.1 * 2.0) as u32);
                let mut rt = runtime(desk, dark);
                let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h);
                let suffix = if language == "en" { "" } else { "-de" };
                let path = folder.join(format!("assistant-desk-{name}{suffix}-{theme}.png"));
                rune::render::write_png(&path, w, h, &pixels).unwrap();
            }
        }
    }
    crate::desktop::i18n::apply(Some("en"));
}
