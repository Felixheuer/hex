//! The quick question's bubble, just past the dictation HUD where Live
//! text and Failure notices show: the question as heard, "Thinking…", the
//! latest steps, then the answer as it streams. A long answer is cut with
//! "…" and the footer's "Show in Assistant" shows the rest. A failed one
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
//! rail's cards); where it hangs (under the island) is the island's.

use rune::{Cx, El, Transition, Vec2, col, el, icon, row, text};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::board::{Bubble, BubblePhase};
use super::failure::{Failure, Remedy};
use super::look::{self, Card, Kind, State, Step};
use super::palette;
use crate::desktop::i18n::{tr, tr_fill, tr_in};

/// One speaking pulse, in seconds: about a syllable pair.
const VOICE_PULSE: f64 = 0.9;

/// The overlay: a readable measure, and room for about ten lines.
pub(crate) const SIZE: (f32, f32) = (440.0, 340.0);
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
    /// "Show in Assistant": open this conversation there.
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
    pub(crate) actions: Actions<BubbleOverlay>,
}

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
    } else {
        state
            .bubble
            .as_ref()
            .map(|bubble| card(p, bubble, voice_now, &state.answer_key, state.actions))
    };
    let motion = if flying {
        fly(state.below)
    } else {
        grow(state.below)
    };
    root.children(shown.map(|card| card.transition(motion)))
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
    let mut body: Vec<El> = match bubble.phase {
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
    if bubble.over() {
        match bubble.failure {
            Some(failure) => card.primary = Some(remedy(p, bubble, failure.remedy(), actions)),
            None => {
                let open = actions.open;
                let id = bubble.conversation.clone();
                card.hint = Some(tr_fill("Hold {} to ask more", answer_key));
                card.secondary.push(
                    look::button(
                        p,
                        Kind::Secondary,
                        tr("Show in Assistant"),
                        move |s: &mut S| open(s, &id),
                    )
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
