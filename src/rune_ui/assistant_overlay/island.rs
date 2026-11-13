//! The island (`docs/ux/redesign.md` §2–3): the conversation's one place.
//! A capsule that is **always black**, in light and dark alike, because it
//! is the notch grown down (or, without a notch, the HUD's capsule under
//! the menu bar); it carries one line of state in white: ≋ "Listening…"
//! while the Action shortcut is held, ✦ "Understood: …" while it thinks,
//! the step it is on while it works, ? who needs the person, ✓ the result
//! in a phrase, ✗ what failed.
//!
//! Under it hangs one card on the one anatomy (`look::card`): while the
//! shortcut is held, the **live words** as they are recognised
//! ([`held_card`]); after, the Here bubble (`bubble::card`) with the
//! reasoning lines (`quick::Bubble::reasoning`).
//!
//! What the island says is plain state ([`line`]), so every rule is a unit
//! test; [`capsule`] draws it.

use rune::{Color, El, LiveRegion, el, icon, row, text};

use super::super::controls::Paint;
use super::failure::Failure;
use super::frame::Tab;
use super::look::{self, Card, State};
use super::quick::{Bubble, BubblePhase, Held, Tag};
use crate::desktop::i18n::{tr, tr_fill, tr_in};

/// The island's fill, in both appearances.
pub(crate) const FILL: Color = Color::BLACK;
/// The island's line strip: its height below the notch.
pub(crate) const HEIGHT: f32 = 36.0;
/// The line's most characters (§2: one sentence per surface).
pub(crate) const LINE_CHARS: usize = 48;
/// The island's narrowest and widest.
pub(crate) const WIDTH: (f32, f32) = (300.0, 420.0);
/// Its corners: the notch's own at the top, round at the bottom.
const RADIUS: f32 = 18.0;
/// Room either side of its line.
const PAD_X: f32 = 18.0;

/// Below the menu bar to a free capsule (a display without a notch).
pub(crate) const FREE_GAP: f32 = 6.0;

/// Where the island hangs on a display (`docs/ux/redesign.md` §3.1), in
/// points from the display's top: grown out of the notch (`notch`, its
/// height) on a display with one, else a free capsule under the menu bar
/// (`bar` tall) at its centre. Its capsule's top, the black above its line
/// (the notch's; 0 for a free capsule), and its bottom.
pub(crate) fn hang(notch: Option<f32>, bar: f32) -> (f32, f32, f32) {
    match notch {
        Some(notch) => (0.0, notch, notch + HEIGHT),
        None => (bar + FREE_GAP, 0.0, bar + FREE_GAP + HEIGHT),
    }
}

/// One line of the island: its state (glyph and colour), its words, and a
/// faint note at its right end ("Mail in front", "2/5").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) state: State,
    pub(crate) words: String,
    pub(crate) trail: Option<String>,
}

/// What the island says: while the shortcut is held, that it listens (and
/// which app is in front); else what the Here question does now. `None`:
/// the island is the notch alone.
#[cfg(test)]
pub(crate) fn line(held: Option<&Held>, bubble: Option<&Bubble>) -> Option<Line> {
    line_with(held, None, bubble)
}

/// [`line`] with the front's moment (`frame::Tab`): a task's window brought
/// in front ("Agent-Desk is using Numbers for a moment") and the front
/// given back ("Back to you · Mail") are the island's to say, before the
/// quick turn's own line; the frame's tab under it keeps only why, Esc,
/// and the countdown. A hold still comes first: the person is talking.
pub(crate) fn line_with(
    held: Option<&Held>,
    front: Option<&Tab>,
    bubble: Option<&Bubble>,
) -> Option<Line> {
    if let Some(held) = held {
        // Which app is in front, and the project the words go to.
        let trail = [
            held.app.as_deref().map(|app| tr_fill("{} in front", app)),
            held.project.as_deref().map(|name| format!("● {name}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        return Some(Line {
            state: State::Listening,
            words: tr("Listening…").to_owned(),
            trail: (!trail.is_empty()).then_some(trail),
        });
    }
    if let Some(front) = front.and_then(front_line) {
        return Some(front);
    }
    let bubble = bubble?;
    if bubble.asks.is_some() {
        return Some(Line {
            state: State::NeedsYou,
            words: tr_fill("{} needs you", &cut(&bubble.question, 28)),
            trail: None,
        });
    }
    let line = match bubble.phase {
        BubblePhase::Thinking => Line {
            state: State::Thinking,
            words: match &bubble.understood {
                Some(understood) => tr_fill("Understood: {}", understood),
                None => tr("Thinking…").to_owned(),
            },
            trail: None,
        },
        BubblePhase::Working => {
            let reasoning = bubble.reasoning();
            let now = reasoning
                .iter()
                .find(|reason| reason.tag == Tag::Now)
                .map(|reason| reason.words.clone())
                .or_else(|| bubble.steps.last().cloned())
                .unwrap_or_else(|| tr("Thinking…").to_owned());
            let done = bubble.plan.iter().filter(|step| step.done).count();
            Line {
                state: State::Working,
                words: now,
                trail: (!bubble.plan.is_empty()).then(|| {
                    format!(
                        "{}/{}",
                        (done + 1).min(bubble.plan.len()),
                        bubble.plan.len()
                    )
                }),
            }
        }
        BubblePhase::Answering => Line {
            state: State::Working,
            words: bubble
                .understood
                .clone()
                .unwrap_or_else(|| bubble.question.clone()),
            trail: None,
        },
        BubblePhase::Answered => Line {
            state: State::Done,
            words: phrase(&bubble.answer).unwrap_or_else(|| tr("Done").to_owned()),
            trail: None,
        },
        BubblePhase::Failed => Line {
            state: State::Failed,
            words: tr_in(
                bubble.language,
                bubble.failure.unwrap_or(Failure::Crash).line(),
            )
            .to_owned(),
            trail: None,
        },
    };
    Some(Line {
        words: cut(&line.words, LINE_CHARS),
        ..line
    })
}

/// The island's line for the front's moment; `None` for real input
/// (`Tab::Hands`), which the frame's tab says alone.
pub(crate) fn front_line(tab: &Tab) -> Option<Line> {
    match tab {
        Tab::InFront { .. } | Tab::Back { .. } => Some(Line {
            state: if tab.in_front() {
                State::InFront
            } else {
                State::Done
            },
            words: cut(&tab.line(), LINE_CHARS),
            trail: None,
        }),
        Tab::Hands { .. } => None,
    }
}

/// An answer's first sentence (or line), plain, as the island's phrase.
fn phrase(answer: &str) -> Option<String> {
    let first = answer
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '-', '*', '>', ' ']))
        .find(|line| !line.is_empty())?;
    let plain: String = first
        .chars()
        .filter(|c| !matches!(c, '*' | '`' | '_'))
        .collect();
    let sentence = plain
        .char_indices()
        .find(|(at, c)| {
            matches!(c, '.' | '!' | '?' | '。')
                && plain[at + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map_or(plain.as_str(), |(at, c)| &plain[..at + c.len_utf8()]);
    Some(cut(sentence.trim(), LINE_CHARS))
}

/// `text` cut to `max` characters with "…".
fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// The state's glyph, lit for the black island (the dark appearance's
/// colours read on black in either).
fn glyph(state: State) -> El {
    let colour = state.colour(true);
    let size = if state == State::Listening {
        16.0
    } else {
        14.0
    };
    icon(state.glyph(), size)
        .key("island-glyph")
        .color(colour)
        .a11y_hidden()
}

/// How wide the island is for `line`: wide enough for its words, within
/// [`WIDTH`].
pub(crate) fn width(line: &Line) -> f32 {
    let words = line.words.chars().count() as f32 * 7.4;
    let trail = line
        .trail
        .as_ref()
        .map_or(0.0, |trail| 12.0 + trail.chars().count() as f32 * 6.2);
    (2.0 * PAD_X + 24.0 + words + trail).clamp(WIDTH.0, WIDTH.1)
}

/// The island with `line`: black, `width` wide; `notch` points of black
/// above the line (the notch it grows from; 0 for a free capsule, rounded
/// all round). A live region: VoiceOver reads each new line politely.
pub(crate) fn capsule(p: Paint, line: &Line, width: f32, notch: f32) -> El {
    let white = Color::WHITE;
    let spoken = format!("{}: {}", line.state.word(), line.words);
    let shape = el()
        .key("island")
        .w(width)
        .h(notch + HEIGHT)
        .fill(FILL)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 6.0,
            blur: 18.0,
            spread: 0.0,
            color: Color::BLACK.alpha(if p.k.dark { 0.5 } else { 0.25 }),
            inset: false,
        })
        .role("status")
        .label(spoken)
        .live_region(LiveRegion::Polite);
    let shape = if notch > 0.0 {
        shape.corners([0.0, 0.0, RADIUS, RADIUS])
    } else {
        shape.radius(HEIGHT / 2.0)
    };
    shape.child(
        row()
            .key("island-line")
            .absolute(PAD_X, notch)
            .size(width - 2.0 * PAD_X, HEIGHT)
            .gap(9.0)
            .center_y()
            .child(glyph(line.state))
            .child(
                text(line.words.clone())
                    .key("island-words")
                    .font(look::island_line(p))
                    .color(white)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0)
                    .a11y_hidden(),
            )
            .children(line.trail.clone().map(|trail| {
                text(trail)
                    .key("island-trail")
                    .font(look::meta(p))
                    .color(white.alpha(0.6))
                    .a11y_hidden()
            })),
    )
}

/// The notch alone, idle: `size` of black, rounded at the bottom.
pub(crate) fn notch(size: (f32, f32)) -> El {
    el().key("notch")
        .size(size.0, size.1)
        .corners([0.0, 0.0, 10.0, 10.0])
        .fill(FILL)
        .a11y_hidden()
}

/// The card under the island while the Action shortcut is held: ≋ and
/// which app is in front, the words heard so far in title size (the last,
/// still tentative ones fainter), and how to send or cancel. A mis-hearing
/// shows before the release.
pub(crate) fn held_card(p: Paint, held: &Held, answer_key: &str) -> El {
    let words = if held.silent() {
        text(tr("Say what you need").to_owned())
            .key("live-words")
            .font(look::title(p))
            .color(p.faint())
            .wrap()
            .w_full()
    } else {
        let mut said = held.words.clone();
        if !held.tentative.is_empty() {
            if !said.is_empty() {
                said.push(' ');
            }
            said.push_str(&held.tentative);
        }
        text(said)
            .key("live-words")
            .font(look::title(p))
            .color(p.text())
            .wrap()
            .max_lines(4)
            .w_full()
    };
    let mut card = Card::named("held");
    card.lead = Some(
        icon(&look::WAVE, 15.0)
            .key("held-wave")
            .color(super::palette::agent(p.k.dark))
            .a11y_hidden(),
    );
    card.title = match &held.app {
        Some(app) => tr_fill("Listening · {} is in front", app),
        None => tr("Listening…").to_owned(),
    };
    card.body = vec![words];
    card.hint = Some(tr_fill("Release {} to send · Esc to cancel", answer_key));
    look::card(p, card)
        .key("held-card")
        .role("status")
        .label(if held.silent() {
            tr("Listening…").to_owned()
        } else {
            held.words.clone()
        })
        .live_region(LiveRegion::Polite)
        .animate_layout()
}

#[cfg(test)]
#[path = "island_tests.rs"]
mod island_tests;

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::assistant::{Event, Origin, PlanStep};

    fn bubble(events: &[Event]) -> Option<Bubble> {
        let now = Instant::now();
        let mut bubble = None;
        Bubble::apply(
            &mut bubble,
            &Event::Started {
                conversation: "q".into(),
                origin: Origin::ActionShortcut,
                question: "Book Nopa for two at eight".into(),
            },
            now,
        );
        for event in events {
            Bubble::apply(&mut bubble, event, now);
        }
        bubble
    }

    #[test]
    fn the_island_says_one_line_for_every_state() {
        crate::desktop::i18n::apply(Some("en"));
        assert_eq!(line(None, None), None, "idle: the notch alone");
        let held = Held::new(Some("Mail".into()));
        let listening = line(Some(&held), None).unwrap();
        assert_eq!(listening.state, State::Listening);
        assert_eq!(listening.trail.as_deref(), Some("Mail in front"));
        let thinking = line(None, bubble(&[]).as_ref()).unwrap();
        assert_eq!(
            (thinking.state, thinking.words.as_str()),
            (State::Thinking, "Thinking…")
        );
        let understood = Event::Understood {
            conversation: "q".into(),
            text: "a table for two".into(),
        };
        let thought = line(None, bubble(std::slice::from_ref(&understood)).as_ref()).unwrap();
        assert_eq!(thought.words, "Understood: a table for two");
        let plan = Event::Plan {
            conversation: "q".into(),
            steps: ["Open Resy", "Pick 8 pm", "Ask before paying"]
                .iter()
                .enumerate()
                .map(|(index, title)| PlanStep {
                    title: (*title).into(),
                    done: index < 1,
                })
                .collect(),
        };
        let step = Event::Step {
            conversation: "q".into(),
            description: "Opened Resy".into(),
        };
        let acting = line(None, bubble(&[understood, plan, step]).as_ref()).unwrap();
        assert_eq!(acting.state, State::Working);
        assert_eq!(acting.words, "Pick 8 pm");
        assert_eq!(acting.trail.as_deref(), Some("2/3"));
        let mut asking = bubble(&[]).unwrap();
        asking.asks = Some("8:15 or 8:45?".into());
        let asks = line(None, Some(&asking)).unwrap();
        assert_eq!(asks.state, State::NeedsYou);
        let done = line(
            None,
            bubble(&[Event::Answer {
                conversation: "q".into(),
                origin: Origin::ActionShortcut,
                text: "**Booked** Nopa for 8:15 pm. It's in your calendar.".into(),
            }])
            .as_ref(),
        )
        .unwrap();
        assert_eq!(
            (done.state, done.words.as_str()),
            (State::Done, "Booked Nopa for 8:15 pm.")
        );
        let failed = line(
            None,
            bubble(&[Event::Failed {
                conversation: "q".into(),
                message: "The task stopped unexpectedly.".into(),
            }])
            .as_ref(),
        )
        .unwrap();
        assert_eq!(failed.state, State::Failed);
        for each in [listening, thinking, thought, acting, asks, done, failed] {
            assert!(each.words.chars().count() <= LINE_CHARS, "{each:?}");
        }
    }

    #[test]
    fn the_island_says_the_fronts_moment_before_the_quick_turn() {
        crate::desktop::i18n::apply(Some("en"));
        let window = crate::assistant::TaskWindow {
            app: "Numbers".into(),
            window_id: Some("w-7".into()),
        };
        let in_front = Tab::InFront {
            app: "Numbers".into(),
            window,
            why: String::new(),
            until_ms: 0,
        };
        let thinking = bubble(&[]);
        let line = line_with(None, Some(&in_front), thinking.as_ref()).unwrap();
        assert_eq!(line.state, State::InFront);
        assert_eq!(line.words, "Agent-Desk is using Numbers for a moment");
        let back = Tab::Back {
            app: "Mail".into(),
            until: Instant::now(),
        };
        let line = line_with(None, Some(&back), None).unwrap();
        assert_eq!(
            (line.state, line.words.as_str()),
            (State::Done, "Back to you · Mail")
        );
        let hands = Tab::Hands {
            app: None,
            window: None,
        };
        assert_eq!(
            line_with(None, Some(&hands), thinking.as_ref())
                .unwrap()
                .state,
            State::Thinking,
            "real input is the frame tab's alone"
        );
        let held = Held::new(None);
        assert_eq!(
            line_with(Some(&held), Some(&in_front), None).unwrap().state,
            State::Listening,
            "the person talking comes first"
        );
    }

    #[test]
    fn the_island_grows_from_the_notch_or_hangs_under_the_menu_bar() {
        // A 32-point notch: from the display's top, its black above.
        assert_eq!(hang(Some(32.0), 32.0), (0.0, 32.0, 32.0 + HEIGHT));
        // No notch: a free capsule under a 24-point menu bar.
        let (top, black, bottom) = hang(None, 24.0);
        assert_eq!((top, black), (24.0 + FREE_GAP, 0.0));
        assert_eq!(bottom, top + HEIGHT);
    }

    #[test]
    fn the_island_is_black_in_both_appearances() {
        // The fill is a constant, never a theme token; the snapshot test
        // checks the pixel at its centre too.
        assert_eq!(FILL, Color::BLACK);
    }
}
