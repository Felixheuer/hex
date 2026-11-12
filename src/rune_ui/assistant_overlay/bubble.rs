//! The quick question's bubble, just past the dictation HUD where Live
//! text and Failure notices show: the question as heard, "Thinking…", the
//! latest steps, then the answer as it streams. A long answer is cut with
//! "…" and the footer's "Open in Chats" shows the rest. It stays while the
//! answer is read aloud, then a few seconds more (`board::Bubble::due`);
//! a press, or Esc, dismisses it.
//!
//! It grows out of the HUD on a spring and shrinks back into it when it
//! goes; while its answer is read aloud a small glow beside the question
//! pulses with a speaking rhythm.

use rune::{Cx, El, Transition, Vec2, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Spinner};
use rune_motion::{LoopSpec, Spring};

use super::super::controls::Paint;
use super::board::{Bubble, BubblePhase};
use super::palette;
use crate::desktop::i18n::tr;

/// One speaking pulse, in seconds: about a syllable pair.
const VOICE_PULSE: f64 = 0.9;

/// The overlay: a readable measure, and room for about ten lines.
pub(crate) const SIZE: (f32, f32) = (440.0, 340.0);
/// About ten lines of answer; past it the answer is cut with "…" and the
/// rest is in Chats.
const ANSWER_CHARS: usize = 560;
/// The answer's tallest, should a short answer still run long (a list).
const ANSWER_MAX_H: f32 = 236.0;
/// The error's most lines.
const ERROR_LINES: usize = 4;
/// Room around the card inside the overlay.
pub(crate) const INSET: f32 = 6.0;

/// What a press asks of the host.
pub(crate) struct Actions<S> {
    /// The card: dismiss it.
    pub(crate) dismiss: fn(&mut S),
    /// "Open in Chats": open this conversation there.
    pub(crate) open: fn(&mut S, &str),
    /// "Continue setup": open the assistant's setup.
    pub(crate) setup: fn(&mut S),
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
/// Action shortcut gets before the assistant is set up.
pub(crate) fn setup_card<S: 'static>(p: Paint, actions: Actions<S>) -> El {
    let c = &p.k.c;
    let setup = actions.setup;
    let dismiss = actions.dismiss;
    let (lead, _) = palette::frame(p.k.dark);
    row()
        .key("setup-card")
        .w_full()
        .pl(16.0)
        .pr(8.0)
        .py(10.0)
        .gap(10.0)
        .center_y()
        .radius(p.k.radius.md + 8.0)
        .fill(c.bg_base)
        .border(p.hair, c.border_base)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .hit()
        .on_press(move |s: &mut S| dismiss(s))
        .animate_layout()
        .child(
            icon(&super::super::icons::SPARKLES, 15.0)
                .key("setup-sparkle")
                .color(lead)
                .a11y_hidden(),
        )
        .child(
            text(tr("Finish setting up the assistant first"))
                .key("setup-text")
                .wrap()
                .font(p.k.ty.ui)
                .color(p.text())
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            Button::new(tr("Continue setup"))
                .variant(ButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut S| setup(s))
                .build(p.k)
                .key("continue-setup"),
        )
}

/// Whether the answer is read aloud now, and whether to hold its pulse
/// still (Reduce motion).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Voice {
    pub(crate) speaking: bool,
    pub(crate) still: bool,
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

/// A small glow that pulses while the answer is read aloud.
fn voice(p: Paint, still: bool) -> El {
    let (lead, trail) = palette::frame(p.k.dark);
    let glow = el()
        .key("voice")
        .size(10.0, 10.0)
        .radius(5.0)
        .gradient(lead, trail)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 8.0,
            spread: 1.0,
            color: lead.alpha(0.6),
            inset: false,
        })
        .a11y_hidden();
    if still {
        glow
    } else {
        glow.looping(LoopSpec::opacity(VOICE_PULSE, 0.0, 0.35, 1.0))
    }
}

/// The card for `bubble`, as wide as its parent allows.
pub(crate) fn card<S: 'static>(
    p: Paint,
    bubble: &Bubble,
    voice_now: Voice,
    answer_key: &str,
    actions: Actions<S>,
) -> El {
    let c = &p.k.c;
    let question = row()
        .key("question-row")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(
            text(bubble.question.clone())
                .key("question")
                .truncate()
                .font(p.k.ty.small)
                .color(p.faint())
                .grow(1.0)
                .min_w(0.0),
        )
        .children(voice_now.speaking.then(|| voice(p, voice_now.still)));
    let body: Vec<El> = match bubble.phase {
        BubblePhase::Thinking => vec![working(p, tr("Thinking…"), "thinking")],
        BubblePhase::Working => {
            let (last, earlier) = bubble
                .steps
                .split_last()
                .map_or((None, &[][..]), |(last, earlier)| (Some(last), earlier));
            earlier
                .iter()
                .enumerate()
                .map(|(index, step)| done_step(p, step, index))
                .chain(last.map(|step| working(p, step, "step")))
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
        BubblePhase::Failed => vec![
            row()
                .key("failure")
                .gap(8.0)
                .items_start()
                .child(
                    icon(p.k.glyph.circle_x, 16.0)
                        .key("failure-icon")
                        .color(p.danger())
                        .mt(2.0)
                        .a11y_hidden(),
                )
                .child(
                    text(bubble.error.clone().unwrap_or_default())
                        .key("error")
                        .wrap()
                        .max_lines(ERROR_LINES)
                        .font(p.k.ty.ui)
                        .color(p.danger())
                        .grow(1.0)
                        .min_w(0.0),
                ),
        ],
    };
    let open = actions.open;
    let id = bubble.conversation.clone();
    let footer = bubble.over().then(|| {
        row().key("footer").w_full().justify_end().child(
            Button::new(tr("Open in Chats"))
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut S| open(s, &id))
                .build(p.k)
                .key("open-chats"),
        )
    });
    let dismiss = actions.dismiss;
    let live = matches!(
        bubble.phase,
        BubblePhase::Thinking | BubblePhase::Working | BubblePhase::Answering
    );
    col()
        .key("card")
        .w_full()
        .px(16.0)
        .pt(12.0)
        .pb(if footer.is_some() { 8.0 } else { 14.0 })
        .gap(8.0)
        .radius(p.k.radius.md + 8.0)
        .fill(c.bg_base)
        .border(
            p.hair,
            if voice_now.speaking {
                palette::frame(p.k.dark).0.alpha(0.6)
            } else {
                c.border_base
            },
        )
        .role("status")
        .label(bubble.question.clone())
        .live_region(if live {
            rune::LiveRegion::Polite
        } else {
            rune::LiveRegion::Off
        })
        .hit()
        .on_press(move |s: &mut S| dismiss(s))
        .animate_layout()
        .child(question)
        .children(body)
        .children(bubble.asks.as_ref().map(|asks| asking(p, asks, answer_key)))
        .children(footer)
}

/// What the task asks the person, with how to answer.
fn asking(p: Paint, asks: &str, answer_key: &str) -> El {
    let (lead, _) = palette::frame(p.k.dark);
    col()
        .key("asks")
        .w_full()
        .gap(3.0)
        .pl(10.0)
        .border_sides([0.0, 0.0, 0.0, 2.0], lead)
        .child(
            text(asks.to_owned())
                .key("asks-text")
                .wrap()
                .max_lines(4)
                .font(p.k.ty.ui_medium)
                .color(p.text()),
        )
        .child(
            text(crate::desktop::i18n::tr_fill(
                "Hold {} to answer",
                answer_key,
            ))
            .key("asks-hint")
            .font(p.k.ty.small)
            .color(lead),
        )
}

/// A line with the activity spinner: what it's doing now.
fn working(p: Paint, line: &str, key: &str) -> El {
    row()
        .key(key)
        .gap(8.0)
        .center_y()
        .child(Spinner::new().size(14.0).build(p.k).key("spinner"))
        .child(
            text(line.to_owned())
                .key("working-text")
                .truncate()
                .font(p.k.ty.ui)
                .color(p.muted())
                .min_w(0.0),
        )
}

/// A step done before the current one.
fn done_step(p: Paint, line: &str, index: usize) -> El {
    row()
        .key(format!("done-{index}"))
        .gap(8.0)
        .center_y()
        .child(
            el().key("bullet")
                .size(14.0, 14.0)
                .center()
                .child(el().size(4.0, 4.0).radius(2.0).fill(p.k.c.icon_faint))
                .a11y_hidden(),
        )
        .child(
            text(line.to_owned())
                .key("done-text")
                .truncate()
                .font(p.k.ty.ui)
                .color(p.faint())
                .min_w(0.0),
        )
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
