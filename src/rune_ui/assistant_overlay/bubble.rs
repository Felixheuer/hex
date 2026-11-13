//! The quick question's bubble, just past the dictation HUD where Live
//! text and Failure notices show: the question as heard, "Thinking…", the
//! latest steps, then the answer as it streams. A long answer is cut with
//! "…" and the footer's More projects the rest on the stage. A failed one
//! says why in a short line, in the language it was asked in
//! ([`super::failure`]), with its one action: Fix (the settings or the
//! setup page that repairs it) or Try again. It stays while the answer is
//! read aloud, then a few seconds more (`board::Bubble::due`); a press, or
//! Esc, dismisses it.
//!
//! It grows out of the HUD on a spring and shrinks back into it when it
//! goes; while its answer is read aloud a small glow in place of its ✦
//! pulses with a speaking rhythm.
//!
//! Its drawing is the one card anatomy (`super::look`, owned with the
//! rail's cards); where it hangs (under the island) is the island's: the
//! panel carries the black island's line at its top (`super::island`) once
//! the HUD has gone, the card under it. While the Action shortcut is still
//! held the HUD is the island, and the card shows the live words instead
//! (`island::held_card`). While the question thinks and works, the card
//! shows the agent's reasoning: Understood, Plan, Now, Next, each ticked as
//! it passes (`quick::Bubble::reasoning`); nothing spins.

use rune::{Cx, El, Transition, Vec2, col, el, icon, row, text};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::board::{Bubble, BubblePhase};
use super::failure::{Failure, Remedy};
use super::island;
use super::look::{self, Card, Kind, State, Step};
use super::palette;
use super::quick::{Held, Mark, Tag};
use crate::desktop::i18n::{tr, tr_fill, tr_in};

/// One speaking pulse, in seconds: about a syllable pair.
const VOICE_PULSE: f64 = 0.9;

/// Between the island and the card under it.
pub(crate) const ISLAND_GAP: f32 = 8.0;
/// The overlay: the island's card width, room for about ten lines, and the
/// island's line above them.
pub(crate) const SIZE: (f32, f32) = (
    look::ISLAND_W + 2.0 * INSET,
    340.0 + island::HEIGHT + ISLAND_GAP,
);
/// About ten lines of answer; past it the answer is cut with "…" and the
/// rest is in the Assistant pane.
const ANSWER_CHARS: usize = 560;
/// The answer's tallest, should a short answer still run long (a list).
const ANSWER_MAX_H: f32 = 236.0;
/// The failure line's most lines.
const ERROR_LINES: usize = 3;
/// The brain's own words beneath it, at most.
const DETAIL_LINES: usize = 2;
/// Room around the card inside the overlay.
pub(crate) const INSET: f32 = 6.0;

/// What a press asks of the host.
pub(crate) struct Actions<S> {
    /// The card: dismiss it.
    pub(crate) dismiss: fn(&mut S),
    /// More: the whole answer on the stage.
    pub(crate) open: fn(&mut S, &str),
    /// "Continue setup", or Fix on what the setup repairs: open the
    /// assistant's setup where it fixes it.
    pub(crate) setup: fn(&mut S),
    /// Fix on a switched-off assistant: Assistant settings.
    pub(crate) settings: fn(&mut S),
    /// Try again: the question (in full) once more, in its conversation.
    pub(crate) retry: fn(&mut S, &str, &str),
}

impl<S> Clone for Actions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Actions<S> {}

/// The bubble overlay's state.
pub(crate) struct BubbleOverlay {
    pub(crate) bubble: Option<Bubble>,
    /// The card hangs from the overlay's top (the HUD is above it), else
    /// rises from its bottom.
    pub(crate) below: bool,
    /// Whether it shows; a hidden bubble shrinks its card back as the
    /// panel fades.
    pub(crate) shown: bool,
    /// Its answer is being read aloud.
    pub(crate) speaking: bool,
    /// How loud the reading sounds now (0 to 1), when it can be measured.
    pub(crate) level: Option<f32>,
    /// "Finish setting up the assistant first" shows instead, until then.
    pub(crate) setup: Option<std::time::Instant>,
    /// The Action shortcut's keys, for "Hold ⌘ to answer".
    pub(crate) answer_key: String,
    /// The shortcut is held: the live words show instead of the bubble.
    pub(crate) held: Option<Held>,
    /// A task's window in front for a moment, or the front just given
    /// back: the island says it (`island::line_with`).
    pub(crate) front: Option<super::frame::Tab>,
    /// Hung from the island (`docs/ux/redesign.md` §3.1) rather than past
    /// the HUD: where in the panel the island and the card go.
    pub(crate) hang: Option<Hang>,
    pub(crate) actions: Actions<BubbleOverlay>,
}

/// The panel hung from the island: the island's capsule at the notch (or
/// under the menu bar's centre), its card where it covers nothing it
/// mustn't (`place::clear_spot`). Points from the panel's top-left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Hang {
    /// The island's middle (x) and its top.
    pub(crate) island: (f32, f32),
    /// The black above the island's line: the notch's height, 0 for a
    /// free capsule.
    pub(crate) notch: f32,
    /// The card's top-left (its inset included).
    pub(crate) card: (f32, f32),
}

/// The panel's widest when hung from the island: room for the card to
/// move aside, clear of what it mustn't cover.
pub(crate) const HUNG_W: f32 = 2.0 * SIZE.0 + 2.0 * INSET;
/// … and its tallest: room to move down past the tab and a label.
pub(crate) const HUNG_H: f32 = SIZE.1 + 200.0;

pub(crate) fn view(state: &BubbleOverlay, cx: &mut Cx<BubbleOverlay>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let root = col()
        .key("bubble-root")
        .w_full()
        .h_full()
        .padding(INSET)
        .items_center()
        .hover_motion_within(rune::Anim::INSTANT, rune::Anim::INSTANT);
    let root = if state.below {
        root.justify_start()
    } else {
        root.justify_end()
    };
    let voice_now = Voice {
        speaking: state.speaking,
        still: cx.system_reduced_motion(),
        level: state.level,
    };
    let flying = state.bubble.as_ref().is_some_and(|b| b.escalated);
    let shown = if !state.shown {
        None
    } else if state.setup.is_some() {
        Some(setup_card(p, state.actions))
    } else if let Some(held) = &state.held {
        Some(island::held_card(p, held, &state.answer_key))
    } else {
        state
            .bubble
            .as_ref()
            .map(|bubble| card(p, bubble, voice_now, &state.answer_key, state.actions))
    };
    // The island's line once the HUD has gone (while held, the HUD is the
    // island); hanging from the top only, as the island does.
    let line = (state.shown && state.below && state.held.is_none() && state.setup.is_none())
        .then(|| island::line_with(None, state.front.as_ref(), state.bubble.as_ref()))
        .flatten();
    let island = line.map(|line| {
        island::capsule(p, &line, island::width(&line), 0.0)
            .mb(ISLAND_GAP)
            .transition(Transition::opacity())
    });
    let motion = if flying {
        fly(state.below)
    } else {
        grow(state.below)
    };
    if let Some(hang) = state.hang {
        // Hung from the island: the capsule at its spot, the card at its.
        let line = (state.shown && state.held.is_none() && state.setup.is_none())
            .then(|| island::line_with(None, state.front.as_ref(), state.bubble.as_ref()))
            .flatten();
        let island = line.map(|line| {
            let width = island::width(&line);
            el().key("island-spot")
                .absolute(hang.island.0 - width / 2.0, hang.island.1)
                .child(
                    island::capsule(p, &line, width, hang.notch).transition(Transition::opacity()),
                )
        });
        let card = shown.map(|card| {
            el().key("card-spot")
                .absolute(hang.card.0, hang.card.1)
                .w(SIZE.0)
                .padding(INSET)
                .child(card.transition(motion))
        });
        return el()
            .key("bubble-root")
            .w_full()
            .h_full()
            .hover_motion_within(rune::Anim::INSTANT, rune::Anim::INSTANT)
            .children(card)
            .children(island);
    }
    root.children(island)
        .children(shown.map(|card| card.transition(motion)))
}

/// About how tall the island's card stands (inside its inset), for
/// keeping it clear of what it mustn't cover before it is laid out
/// (`place::clear_spot`): the live words while `held`, else `bubble`'s
/// card, else the setup card.
pub(crate) fn about_h(bubble: Option<&Bubble>, held: bool) -> f32 {
    // Padding, header, footer, and the gaps between.
    const FRAME: f32 = 2.0 * look::PADDING + 20.0 + look::BUTTON_H + 2.0 * look::GAP;
    const LINE: f32 = 22.0;
    const PER_LINE: f32 = 62.0;
    let lines = |text: &str| (text.chars().count() as f32 / PER_LINE).ceil().max(1.0);
    let body = match bubble {
        _ if held => 4.0 * LINE,
        None => LINE,
        Some(bubble) if bubble.asks.is_some() => {
            lines(bubble.asks.as_deref().unwrap_or_default()) * LINE
        }
        Some(bubble) => match bubble.phase {
            BubblePhase::Answered | BubblePhase::Answering => {
                (lines(&clip_answer(&bubble.answer, ANSWER_CHARS)) * LINE).min(ANSWER_MAX_H)
            }
            BubblePhase::Failed => 3.0 * LINE,
            BubblePhase::Thinking | BubblePhase::Working => {
                bubble.reasoning().len().max(1) as f32 * 26.0
            }
        },
    };
    FRAME + body
}

/// A Here question that became a Background task: the card shrinks to a dot
/// and flies off toward the orb, beside the HUD, as its dot arrives there.
fn fly(below: bool) -> Transition {
    let anchor = Vec2::new(1.0, if below { 0.0 } else { 1.0 });
    Transition::asymmetric(
        Transition::opacity(),
        Transition::scale_anchor(0.5, anchor)
            .allow_small_scale()
            .combined(Transition::offset(240.0, if below { -24.0 } else { 24.0 }))
            .combined(Transition::opacity()),
    )
    .springs(rune::SpringPair::new(
        Spring::new(0.3, 0.0),
        Spring::new(0.42, 0.1),
    ))
}

/// "Finish setting up the assistant first", with Continue setup: what the
/// Action shortcut gets before the assistant is set up, on the one card
/// anatomy (✦, what's missing, Continue setup as its primary).
pub(crate) fn setup_card<S: 'static>(p: Paint, actions: Actions<S>) -> El {
    let setup = actions.setup;
    let dismiss = actions.dismiss;
    let mut card = Card::named("setup");
    card.body = vec![
        row()
            .w_full()
            .gap(10.0)
            .center_y()
            .child(
                icon(&look::SPARK, 13.0)
                    .key("setup-sparkle")
                    .color(palette::agent(p.k.dark))
                    .a11y_hidden(),
            )
            .child(
                // Set up before but missing something now: what.
                look::body_text(
                    p,
                    crate::assistant::setup_gap()
                        .map(tr)
                        .unwrap_or(tr("Finish setting up the assistant first")),
                )
                .key("setup-text")
                .grow(1.0)
                .min_w(0.0),
            ),
    ];
    card.primary = Some(
        look::button(p, Kind::Primary, tr("Continue setup"), move |s: &mut S| {
            setup(s)
        })
        .key("continue-setup"),
    );
    look::card(p, card)
        .key("setup-card")
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .hit()
        .on_press(move |s: &mut S| dismiss(s))
        .animate_layout()
}
/// Whether the answer is read aloud now, how loud (`speech::level`, when it
/// can be measured), and whether to hold its pulse still (Reduce motion).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Voice {
    pub(crate) speaking: bool,
    pub(crate) still: bool,
    pub(crate) level: Option<f32>,
}

/// A voice level (linear, 0 to 1) as the pulse shows it: louder speech
/// grows it, quickly at first (speech sits low on a linear scale), in
/// twentieths so a steady voice doesn't redraw.
pub(crate) fn pulse(level: f32) -> f32 {
    (level.clamp(0.0, 1.0).sqrt() * 20.0).round() / 20.0
}

/// The card grows out of the HUD's side with a little overshoot, and
/// shrinks back into it.
fn grow(below: bool) -> Transition {
    let anchor = Vec2::new(0.5, if below { 0.0 } else { 1.0 });
    Transition::asymmetric(
        Transition::scale_anchor(0.6, anchor)
            .allow_small_scale()
            .combined(Transition::offset(0.0, if below { -8.0 } else { 8.0 })),
        Transition::scale_anchor(0.7, anchor)
            .allow_small_scale()
            .combined(Transition::opacity()),
    )
    .springs(rune::SpringPair::new(
        Spring::new(0.45, 0.24),
        Spring::new(0.26, 0.0),
    ))
}

/// A small glow that pulses while the answer is read aloud: with the
/// voice's level when it can be measured, else on a speaking rhythm.
fn voice(p: Paint, voice: Voice) -> El {
    let (lead, trail) = palette::frame(p.k.dark);
    let size = voice.level.map_or(10.0, |level| 7.0 + 9.0 * pulse(level));
    let glow = el()
        .key("voice")
        .size(16.0, 16.0)
        .center()
        .child(
            el().key("voice-dot")
                .size(size, size)
                .radius(size / 2.0)
                .gradient(lead, trail)
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 8.0,
                    spread: 1.0,
                    color: lead.alpha(0.6),
                    inset: false,
                })
                .opacity(voice.level.map_or(1.0, |level| 0.45 + 0.55 * pulse(level))),
        )
        .a11y_hidden();
    if voice.still || voice.level.is_some() {
        glow
    } else {
        glow.looping(LoopSpec::opacity(VOICE_PULSE, 0.0, 0.35, 1.0))
    }
}

/// The card for `bubble` on the one anatomy (`look::card`), as wide as its
/// parent allows: ✦ (or the speaking glow) and the words the person said
/// with the state chip (Thinking, Working, Needs you, Done, Failed); the
/// steps (✓ past, the agent's dot now: nothing spins), the answer, or why
/// it failed; what the task asks, if it does; and, once it's over, the
/// footer: how to ask more by voice at the left, the one action at the
/// right (Fix or Try again on a failure).
pub(crate) fn card<S: 'static>(
    p: Paint,
    bubble: &Bubble,
    voice_now: Voice,
    answer_key: &str,
    actions: Actions<S>,
) -> El {
    let lead = if voice_now.speaking {
        voice(p, voice_now)
    } else {
        icon(&look::SPARK, 13.0)
            .key("spark")
            .color(palette::agent(p.k.dark))
            .a11y_hidden()
    };
    let state = if bubble.asks.is_some() {
        State::NeedsYou
    } else {
        match bubble.phase {
            BubblePhase::Thinking => State::Thinking,
            BubblePhase::Working | BubblePhase::Answering => State::Working,
            BubblePhase::Answered => State::Done,
            BubblePhase::Failed => State::Failed,
        }
    };
    let reasoning = bubble.reasoning();
    let mut body: Vec<El> = match bubble.phase {
        // Asking while it worked: the question alone, nothing seems to run.
        BubblePhase::Thinking | BubblePhase::Working if bubble.asks.is_some() => Vec::new(),
        _ if !reasoning.is_empty() => reasoning_lines(p, &reasoning),
        BubblePhase::Thinking => {
            vec![look::step_line(p, None, tr("Thinking…"), Step::Now).key("thinking")]
        }
        BubblePhase::Working => {
            let (last, earlier) = bubble
                .steps
                .split_last()
                .map_or((None, &[][..]), |(last, earlier)| (Some(last), earlier));
            earlier
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    look::step_line(p, None, step, Step::Done).key(format!("done-{index}"))
                })
                .chain(last.map(|step| look::step_line(p, None, step, Step::Now).key("step")))
                .collect()
        }
        BubblePhase::Answering | BubblePhase::Answered => vec![
            el().key("answer")
                .w_full()
                .max_h(ANSWER_MAX_H)
                .clip_rounded(0.0)
                .child(super::markdown::view(
                    p,
                    &clip_answer(bubble.answer.trim(), ANSWER_CHARS),
                )),
        ],
        BubblePhase::Failed => vec![failed(p, bubble)],
    };
    body.extend(bubble.asks.as_ref().map(|asks| asking(p, asks, answer_key)));
    let mut card = Card::named("card");
    card.lead = Some(lead);
    card.title = bubble.question.clone();
    card.chip = Some(look::chip(p, state));
    card.body = body;
    if !bubble.over() && bubble.asks.is_none() {
        // Still thinking or working: the person may steer it or stop it.
        card.hint = Some(tr_fill("Hold {} to change it · Esc stops", answer_key));
    }
    if bubble.over() {
        match bubble.failure {
            Some(failure) => card.primary = Some(remedy(p, bubble, failure.remedy(), actions)),
            None => {
                let open = actions.open;
                let id = bubble.conversation.clone();
                card.hint = Some(tr_fill("Hold {} to ask more", answer_key));
                card.secondary.push(
                    look::button(p, Kind::Secondary, tr("More"), move |s: &mut S| {
                        open(s, &id)
                    })
                    .key("open-chats"),
                );
            }
        }
    }
    let dismiss = actions.dismiss;
    let live = matches!(
        bubble.phase,
        BubblePhase::Thinking | BubblePhase::Working | BubblePhase::Answering
    );
    let card = look::card(p, card)
        .key("card")
        .role("status")
        .label(format!("{}: {}", state.word(), bubble.question))
        .live_region(if live {
            rune::LiveRegion::Polite
        } else {
            rune::LiveRegion::Off
        })
        .hit()
        // A click while the turn still works or reads its answer is harmless:
        // it never hides a running turn (the person took that for a cancel).
        // Once it is over, a press puts the bubble away; Esc always does.
        .on_press(move |s: &mut S| {
            if !live {
                dismiss(s);
            }
        })
        .animate_layout();
    if voice_now.speaking {
        // While it reads aloud, the card's edge glows in the agent's hue.
        card.border(p.hair, palette::agent(p.k.dark).alpha(0.6))
    } else {
        card
    }
}

/// The reasoning lines: a small tag (Understood, Plan, Doing now, Up next)
/// and the words, ✓ once passed, the agent's dot on the one now, an empty
/// ring for what comes. Reduce motion changes nothing: no line moves.
fn reasoning_lines(p: Paint, reasoning: &[super::quick::Reason]) -> Vec<El> {
    let mut earlier = 0;
    reasoning
        .iter()
        .map(|reason| {
            let step = match reason.mark {
                Mark::Done => Step::Done,
                Mark::Now => Step::Now,
                Mark::Next => Step::Next,
            };
            let (tag, key) = match reason.tag {
                Tag::Understood => (tr("Understood"), "understood".to_owned()),
                Tag::Plan => (tr("Plan"), "plan".to_owned()),
                Tag::Earlier => {
                    earlier += 1;
                    ("", format!("done-{}", earlier - 1))
                }
                Tag::Now => (tr("Doing now"), "step".to_owned()),
                Tag::Next => (tr("Up next"), "next".to_owned()),
            };
            look::step_line(p, Some(tag), &reason.words, step).key(key)
        })
        .collect()
}

/// Why the question failed: its line in the language it was asked in, and
/// beneath it, small, the brain's own words when they say more (which
/// agent, which tool). The chip says Failed; the line says why.
fn failed(p: Paint, bubble: &Bubble) -> El {
    let failure = bubble.failure.unwrap_or(Failure::Crash);
    let detail = bubble
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| failure.shows_detail() && !error.is_empty());
    col()
        .key("failure")
        .w_full()
        .gap(3.0)
        .child(
            text(tr_in(bubble.language, failure.line()).to_owned())
                .key("error")
                .wrap()
                .max_lines(ERROR_LINES)
                .font(look::body(p))
                .color(p.text()),
        )
        .children(detail.map(|detail| {
            text(detail.to_owned())
                .key("error-detail")
                .wrap()
                .max_lines(DETAIL_LINES)
                .font(look::meta(p))
                .color(p.faint())
        }))
}

/// A failure's one action, the card's primary: Fix (Assistant settings, or
/// the setup where it repairs it) or Try again.
fn remedy<S: 'static>(p: Paint, bubble: &Bubble, remedy: Remedy, actions: Actions<S>) -> El {
    let key = match remedy {
        Remedy::Retry => "failure-retry",
        Remedy::Settings | Remedy::Repair => "failure-fix",
    };
    let label = tr(remedy.label());
    match remedy {
        Remedy::Settings => {
            let settings = actions.settings;
            look::button(p, Kind::Primary, label, move |s: &mut S| settings(s))
        }
        Remedy::Repair => {
            let setup = actions.setup;
            look::button(p, Kind::Primary, label, move |s: &mut S| setup(s))
        }
        Remedy::Retry => {
            let retry = actions.retry;
            let (id, asked) = (bubble.conversation.clone(), bubble.asked.clone());
            look::button(p, Kind::Primary, label, move |s: &mut S| {
                retry(s, &id, &asked)
            })
        }
    }
    .key(key)
}

/// What the task asks the person, with how to answer.
fn asking(p: Paint, asks: &str, answer_key: &str) -> El {
    col()
        .key("asks")
        .w_full()
        .gap(4.0)
        .child(
            text(asks.to_owned())
                .key("asks-text")
                .wrap()
                .max_lines(4)
                .font(look::body(p))
                .color(p.text()),
        )
        .child(look::hint(p, &tr_fill("Hold {} to answer", answer_key)).key("asks-hint"))
}
/// `answer` (Markdown) cut to about `max` characters: at the last line
/// break that fits, else the last word, then "…". Never inside a code
/// span or a bold run left open: their markers are dropped from the cut.
pub(crate) fn clip_answer(answer: &str, max: usize) -> String {
    if answer.chars().count() <= max {
        return answer.to_owned();
    }
    let head: String = answer.chars().take(max).collect();
    let cut = head
        .rfind('\n')
        .filter(|at| *at > max / 2)
        .or_else(|| head.rfind(' '))
        .unwrap_or(head.len());
    let mut clipped = head[..cut]
        .trim_end_matches([' ', ',', ';', ':', '.', '\n'])
        .to_owned();
    // An odd number of a marker leaves its run open past the cut.
    for marker in ["**", "`"] {
        if clipped.matches(marker).count() % 2 == 1
            && let Some(at) = clipped.rfind(marker)
        {
            clipped.replace_range(at..at + marker.len(), "");
        }
    }
    clipped + "…"
}

#[cfg(test)]
mod tests {
    use super::clip_answer;

    #[test]
    fn a_long_answer_is_cut_at_a_line_or_word_without_open_markup() {
        assert_eq!(clip_answer("Short.", 20), "Short.");
        assert_eq!(clip_answer("one two three four five", 12), "one two…");
        let list = "- first item here\n- second item here\n- third item";
        assert_eq!(
            clip_answer(list, 40),
            "- first item here\n- second item here…"
        );
        let bold = "Use **this very long bold phrase that keeps going** now";
        let cut = clip_answer(bold, 30);
        assert_eq!(cut.matches("**").count() % 2, 0, "{cut}");
        let code = "Run `cargo build --locked --release` to build";
        assert_eq!(clip_answer(code, 24).matches('`').count() % 2, 0);
    }
}
