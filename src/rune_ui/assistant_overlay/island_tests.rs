//! The island and the card under it, laid out offscreen in every state,
//! and (ignored, with a GPU) rendered to
//! `assistant-island-<state>-{light,dark}.png`, with a pixel check that the
//! island is black in both appearances.

use std::time::Instant;

use rune::text::TextSystem;
use rune::{Cx, El, Env, Runtime, col, el, row, text};

use super::super::super::controls::Paint;
use super::super::bubble::{self, BubbleOverlay};
use super::super::fixture;
use super::super::quick::{Bubble, Held};
use super::*;
use crate::assistant::{Event, Origin, PlanStep};

/// A slice of the desktop's top: the menu bar, the notch, the island, and
/// the card under it.
const SCENE: (f32, f32) = (720.0, 420.0);
const MENU_BAR: f32 = 30.0;
const NOTCH: (f32, f32) = (180.0, MENU_BAR);

/// What a scene shows.
#[derive(Clone)]
struct Scene {
    held: Option<Held>,
    bubble: Option<Bubble>,
    front: Option<super::super::frame::Tab>,
}

fn scene_view(scene: &Scene, cx: &mut Cx<Scene>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let c = &p.k.c;
    let mut root = el()
        .key("scene")
        .size(SCENE.0, SCENE.1)
        .fill(c.bg_deep)
        .clip_rounded(0.0)
        .child(
            row()
                .absolute(0.0, 0.0)
                .size(SCENE.0, MENU_BAR)
                .px(16.0)
                .gap(18.0)
                .center_y()
                .fill(c.bg_base)
                .border_sides([0.0, 0.0, p.hair, 0.0], c.border_base)
                .children(
                    ["Mail", "File", "Edit", "View"]
                        .map(|word| text(word).font(p.k.ty.small_medium).color(p.muted())),
                ),
        );
    let shown = line_with(
        scene.held.as_ref(),
        scene.front.as_ref(),
        scene.bubble.as_ref(),
    );
    let bottom = match &shown {
        Some(shown) => {
            let width = width(shown);
            root = root.child(
                el().absolute((SCENE.0 - width) / 2.0, 0.0)
                    .child(capsule(p, shown, width, MENU_BAR)),
            );
            MENU_BAR + HEIGHT
        }
        None => {
            root = root.child(
                el().absolute((SCENE.0 - NOTCH.0) / 2.0, 0.0)
                    .child(notch(NOTCH)),
            );
            NOTCH.1
        }
    };
    // The frame's tab under the island while a window is in front: why,
    // Esc, and the countdown (the island says the rest).
    let tab = scene
        .front
        .as_ref()
        .filter(|tab| tab.in_front())
        .map(|tab| {
            row()
                .absolute(0.0, bottom + 8.0)
                .w(SCENE.0)
                .justify_center()
                .child(super::super::frame::tab_view(p, tab, Some(2)))
        });
    root = root.children(tab);
    let actions = bubble::Actions::<Scene> {
        dismiss: |_| {},
        open: |_, _| {},
        setup: |_| {},
        settings: |_| {},
        retry: |_, _, _| {},
        take_project: |_, _| {},
    };
    let card = match (&scene.held, &scene.bubble) {
        (Some(held), _) => Some(held_card(p, held, fixture::ANSWER_KEY)),
        (None, Some(shown)) => Some(bubble::card(
            p,
            shown,
            bubble::Voice::default(),
            fixture::ANSWER_KEY,
            actions,
        )),
        (None, None) => None,
    };
    root.children(card.map(|card| {
        col()
            .absolute(
                (SCENE.0 - super::super::look::ISLAND_W) / 2.0,
                bottom + 10.0,
            )
            .w(super::super::look::ISLAND_W)
            .child(card)
    }))
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

fn text_of<S>(rt: &Runtime<S>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .and_then(|e| e.text)
}

const QUESTION: &str = "Book Nopa for two at eight";

fn quick(events: Vec<Event>) -> Option<Bubble> {
    let now = Instant::now();
    let mut bubble = None;
    let started = Event::Started {
        conversation: "nopa".into(),
        origin: Origin::ActionShortcut,
        question: QUESTION.into(),
    };
    for event in std::iter::once(started).chain(events) {
        Bubble::apply(&mut bubble, &event, now);
    }
    bubble
}

fn understood() -> Event {
    Event::Understood {
        conversation: "nopa".into(),
        text: "Nopa, 2 people, tonight around 8 pm".into(),
    }
}

fn plan(done: usize) -> Event {
    Event::Plan {
        conversation: "nopa".into(),
        steps: [
            "Open Resy in a background browser",
            "Find Nopa tonight for 2",
            "Pick 8 pm or the nearest time",
            "Ask before paying",
        ]
        .iter()
        .enumerate()
        .map(|(index, title)| PlanStep {
            title: (*title).into(),
            done: index < done,
        })
        .collect(),
    }
}

fn step(description: &str) -> Event {
    Event::Step {
        conversation: "nopa".into(),
        description: description.into(),
    }
}

/// Every state the island draws, by name.
fn scenes() -> Vec<(&'static str, Scene)> {
    let mut held = Held::new(Some("Mail".into()));
    held.hear("Book Nopa for two at", "eight");
    let mut asking = quick(vec![understood(), plan(1), step("Opened Resy")]).unwrap();
    asking.asks =
        Some("8:00 is gone. Take 8:15 or 8:45? It charges your saved card 20 € to hold it.".into());
    let mut folded = quick(vec![understood(), plan(2), step("Found Nopa tonight")]).unwrap();
    folded.fold(Instant::now() + super::super::quick::REASONING_FOLDS);
    let none = |bubble| Scene {
        held: None,
        bubble,
        front: None,
    };
    vec![
        ("idle", none(None)),
        (
            "listening",
            Scene {
                held: Some(held),
                bubble: None,
                front: None,
            },
        ),
        (
            "listening-silent",
            Scene {
                held: Some(Held::new(Some("Mail".into()))),
                bubble: None,
                front: None,
            },
        ),
        ("thinking", none(quick(vec![understood(), plan(0)]))),
        (
            "acting",
            none(quick(vec![understood(), plan(1), step("Opened Resy")])),
        ),
        ("acting-folded", none(Some(folded))),
        (
            "acting-steps",
            none(quick(vec![
                understood(),
                step("Opened Calendar"),
                step("Read this afternoon's events"),
            ])),
        ),
        ("asking", none(Some(asking))),
        (
            "borrowed",
            Scene {
                held: None,
                bubble: None,
                front: Some(super::super::frame::Tab::InFront {
                    app: "Numbers".into(),
                    window: crate::assistant::TaskWindow {
                        app: "Numbers".into(),
                        window_id: Some("w-7".into()),
                    },
                    why: crate::assistant::guard::BORROW_IGNORED.into(),
                    until_ms: 0,
                }),
            },
        ),
        (
            "back",
            Scene {
                held: None,
                bubble: None,
                front: Some(super::super::frame::Tab::Back {
                    app: "Mail".into(),
                    until: Instant::now(),
                }),
            },
        ),
        (
            "done",
            none(quick(vec![
                understood(),
                Event::Answer {
                    conversation: "nopa".into(),
                    origin: Origin::ActionShortcut,
                    text: "Booked: a table for two at 8:15 pm tonight. It's in your calendar."
                        .into(),
                },
            ])),
        ),
        (
            "failed",
            none(quick(vec![Event::Failed {
                conversation: "nopa".into(),
                message: "Claude Code isn't signed in. Open Terminal and run “claude” to sign in."
                    .into(),
            }])),
        ),
    ]
}

#[test]
fn every_island_state_lays_out_inside_its_scene() {
    crate::desktop::i18n::apply(Some("en"));
    for dark in [false, true] {
        for (name, scene) in scenes() {
            let rt = runtime(scene, scene_view, SCENE, dark);
            match name {
                "idle" => {
                    assert!(rect(&rt, "island").is_none(), "{name}: the notch alone");
                    assert!(rect(&rt, "notch").is_some());
                }
                _ => {
                    let island = rect(&rt, "island").expect(name);
                    assert!(island[1] <= 0.5, "{name}: hangs from the top");
                    assert!(island[2] <= WIDTH.1 + 0.5, "{name}: {island:?}");
                }
            }
            if let Some([x, y, w, h]) = rect(&rt, "card").or(rect(&rt, "held-card")) {
                assert!(x >= 0.0 && x + w <= SCENE.0, "{name}");
                assert!(y + h <= SCENE.1, "{name}: {h}");
            }
        }
    }
}

#[test]
fn live_words_show_under_the_island_while_held() {
    let mut held = Held::new(Some("Mail".into()));
    held.hear("Book Nopa for two", "at");
    let rt = runtime(
        Scene {
            held: Some(held),
            bubble: None,
            front: None,
        },
        scene_view,
        SCENE,
        false,
    );
    assert_eq!(
        text_of(&rt, "live-words").as_deref(),
        Some("Book Nopa for two at")
    );
    assert_eq!(text_of(&rt, "island-words").as_deref(), Some("Listening…"));
    assert_eq!(
        text_of(&rt, "island-trail").as_deref(),
        Some("Mail in front")
    );
    assert!(rect(&rt, "held-hint").is_some(), "Release ⌘ to send");
}

#[test]
fn the_reasoning_card_shows_understood_before_its_steps() {
    let rt = runtime(
        Scene {
            held: None,
            bubble: quick(vec![understood(), plan(1), step("Opened Resy")]),
            front: None,
        },
        scene_view,
        SCENE,
        false,
    );
    let understood = rect(&rt, "understood").expect("understood");
    let plan = rect(&rt, "plan").expect("plan");
    let now = rect(&rt, "step").expect("now");
    let next = rect(&rt, "next").expect("next");
    assert!(understood[1] < plan[1] && plan[1] < now[1] && now[1] < next[1]);
    assert_eq!(
        text_of(&rt, "island-words").as_deref(),
        Some("Find Nopa tonight for 2"),
        "the island says what it does now"
    );
}

/// The bubble's panel: the island's line above the card once the HUD has
/// gone; while held, the live words and no second island (the HUD is it).
#[test]
fn the_bubble_panel_carries_the_island_line_above_its_card() {
    let state = |held: Option<Held>, bubble: Option<Bubble>| BubbleOverlay {
        bubble,
        below: true,
        shown: true,
        speaking: false,
        setup: None,
        answer_key: fixture::ANSWER_KEY.into(),
        level: None,
        held,
        front: None,
        hang: None,
        actions: bubble::Actions {
            dismiss: |_| {},
            open: |_, _| {},
            setup: |_| {},
            settings: |_| {},
            retry: |_, _, _| {},
            take_project: |_, _| {},
        },
    };
    let rt = runtime(
        state(None, quick(vec![understood()])),
        bubble::view,
        bubble::SIZE,
        true,
    );
    let island = rect(&rt, "island").expect("the island's line");
    let card = rect(&rt, "card").expect("the card");
    assert!(
        island[1] + island[3] <= card[1],
        "{island:?} above {card:?}"
    );
    assert!(card[1] + card[3] <= bubble::SIZE.1 + 0.5);
    let held = runtime(
        state(Some(Held::new(None)), quick(vec![understood()])),
        bubble::view,
        bubble::SIZE,
        true,
    );
    assert!(rect(&held, "island").is_none(), "the HUD is the island");
    assert!(rect(&held, "held-card").is_some());
    assert!(rect(&held, "card").is_none(), "the live words win");
}

/// Renders every island state to `AGENT_DESK_ASSISTANT_SNAPS`, light and
/// dark, and checks the island's pixels are black in both.
#[test]
#[ignore = "set AGENT_DESK_ASSISTANT_SNAPS to a folder; needs a GPU"]
fn island_snapshots() {
    let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_ASSISTANT_SNAPS").unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
    crate::desktop::i18n::apply(Some("en"));
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let (w, h) = ((SCENE.0 * 2.0) as u32, (SCENE.1 * 2.0) as u32);
        for (name, scene) in scenes() {
            let mut rt = runtime(scene, scene_view, SCENE, dark);
            let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, w, h);
            if let Some([x, y, iw, ih]) = rect(&rt, "island") {
                // Inside its left padding, clear of glyph and words.
                let (px, py) = (
                    ((x + 8.0) * 2.0) as u32,
                    ((y + ih - HEIGHT / 2.0) * 2.0) as u32,
                );
                let at = ((py * w + px) * 4) as usize;
                let rgb = &pixels[at..at + 3];
                assert!(
                    rgb.iter().all(|v| *v < 8),
                    "{name} {theme}: the island is black, not {rgb:?} ({iw} wide)"
                );
            }
            let path = folder.join(format!("assistant-island-{name}-{theme}.png"));
            rune::render::write_png(&path, w, h, &pixels).unwrap();
        }
        // The whole desk while held and while it thinks.
        let (dw, dh) = (
            (super::super::desk::SIZE.0 * 2.0) as u32,
            (super::super::desk::SIZE.1 * 2.0) as u32,
        );
        let mut held = Held::new(Some("Mail".into()));
        held.hear(
            "Reply to Maya that Thursday works and book Nopa for two at",
            "",
        );
        let desks = [
            (
                "desktop-island-listening",
                super::super::desk::Desk {
                    held: Some(held),
                    bubble: None,
                    board: fixture::quiet_board(),
                    ..super::super::desk::Desk::settled()
                },
            ),
            (
                "desktop-island-thinking",
                super::super::desk::Desk {
                    bubble: fixture::bubble_at(9.1),
                    board: fixture::quiet_board(),
                    ..super::super::desk::Desk::settled()
                },
            ),
        ];
        for (name, desk) in desks {
            let mut rt = runtime(
                desk,
                super::super::desk::view,
                super::super::desk::SIZE,
                dark,
            );
            let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, dw, dh);
            let path = folder.join(format!("assistant-{name}-{theme}.png"));
            rune::render::write_png(&path, dw, dh, &pixels).unwrap();
        }
    }
}
