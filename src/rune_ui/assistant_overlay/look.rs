//! The overlay's one visual language (`docs/ux/redesign.md` §3): the four
//! type sizes, the card material, the state chip, the pill button, the one
//! bubble anatomy every card shares, and the glyphs they draw. Everything
//! the assistant draws over the desktop (island, rail, stage) builds from
//! these, so a question, a result, a job row, and the Here bubble read as
//! one family.
//!
//! The anatomy ([`card`]):
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ ● Task title (or ✦ the words you said)  [chip] │  header: who · what · state
//! │ Body: one thing, ≤ 4 lines, 14 pt               │
//! │ ✓ Understood  …                                 │  optional reasoning lines
//! │ Hold ⌘ and say it            [Second] [Primary] │  footer: voice hint left, actions right
//! └─────────────────────────────────────────────────┘
//! radius 16 · padding 16 · gap 10 · hairline · one shadow
//! widths: 348 rail · 470 island · 560–640 stage
//! ```
//!
//! Rules the types hold: at most one primary action (inverse fill) and two
//! secondary ones ([`Card::primary`], [`Card::secondary`]); × and pin go in
//! the header's right end ([`Card::trail`]) only on the stage; a state is
//! always a glyph *and* a word ([`State`]), never colour alone.
//!
//! Stable API for the other overlay agents (island, frames, stage): the
//! type ramp ([`title`], [`body`], [`label`], [`meta`], [`island_line`]),
//! [`material`], [`shadow`], [`dot`], [`chip`] / [`chip_words`],
//! [`button`], [`card`] with [`Card`], [`body_text`], [`hint`],
//! [`progress`], [`step_line`], the glyphs, and the widths.

// The island, the frames, and the stage (agents B–D of the redesign) build
// on what the rail does not use yet: keep it, documented, for them.
#![allow(dead_code)]

use rune::{Color, El, Icon, Shadow, TextStyle, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};

use super::super::controls::Paint;
use super::board::Phase;
use super::palette;
use std::time::Duration;

use crate::desktop::i18n::{tr, tr_fill};

// ------------------------------------------------------------------ sizes

/// A card's corner radius.
pub(crate) const RADIUS: f32 = 16.0;
/// Room inside a card.
pub(crate) const PADDING: f32 = 16.0;
/// Between a card's header, body lines, and footer.
pub(crate) const GAP: f32 = 10.0;
/// A rail card's width (questions, results, job rows, the list).
pub(crate) const RAIL_W: f32 = 348.0;
/// The card under the island.
pub(crate) const ISLAND_W: f32 = 470.0;
/// A stage panel's narrowest and widest.
pub(crate) const STAGE_W: (f32, f32) = (560.0, 640.0);
/// A pill button's height.
pub(crate) const BUTTON_H: f32 = 28.0;
/// A chip's height.
pub(crate) const CHIP_H: f32 = 20.0;

// ------------------------------------------------------------------ type

/// Title, 17/22 medium: live words, a result's headline.
pub(crate) fn title(p: Paint) -> TextStyle {
    TextStyle {
        size: 17.0,
        line_height: 22.0,
        ..p.k.ty.ui_medium
    }
}

/// Body, 14/20: what a card says.
pub(crate) fn body(p: Paint) -> TextStyle {
    TextStyle {
        size: 14.0,
        line_height: 20.0,
        ..p.k.ty.ui
    }
}

/// Label, 12/16 medium: card headers, buttons, chips.
pub(crate) fn label(p: Paint) -> TextStyle {
    TextStyle {
        size: 12.0,
        line_height: 16.0,
        ..p.k.ty.small_medium
    }
}

/// Meta, 11.5/15: hints, places, times.
pub(crate) fn meta(p: Paint) -> TextStyle {
    TextStyle {
        size: 11.5,
        line_height: 15.0,
        ..p.k.ty.small
    }
}

/// The island's one line: 13.5 medium (white, on the always-black island).
pub(crate) fn island_line(p: Paint) -> TextStyle {
    TextStyle {
        size: 13.5,
        line_height: 18.0,
        ..p.k.ty.ui_medium
    }
}

// ------------------------------------------------------------------ material

/// The one shadow every card casts (28/10; 16 % light, 45 % dark).
pub(crate) fn shadow(p: Paint) -> Shadow {
    Shadow {
        x: 0.0,
        y: 10.0,
        blur: 28.0,
        spread: 0.0,
        color: Color::BLACK.alpha(if p.k.dark { 0.45 } else { 0.16 }),
        inset: false,
    }
}

/// The card material: the kit's `bg_base`, a hairline, [`RADIUS`], and
/// [`shadow`]. Cards in the rail and under the island wear it; the island
/// itself is always black.
pub(crate) fn material(p: Paint, e: El) -> El {
    let c = &p.k.c;
    e.radius(RADIUS)
        .fill(c.bg_base.alpha(1.0))
        .border(p.hair, c.border_base)
        .box_shadow(shadow(p))
}

/// A round dot in `colour` (a task's, beside its title).
pub(crate) fn dot(colour: Color, size: f32) -> El {
    el().size(size, size)
        .radius(size / 2.0)
        .fill(colour)
        .a11y_hidden()
}

/// A thin plan bar: `part` (0 to 1) of `width` in `colour` on a faint track.
pub(crate) fn progress(p: Paint, colour: Color, part: f32, width: f32) -> El {
    let part = part.clamp(0.0, 1.0);
    el().key("progress")
        .w(width)
        .h(3.0)
        .radius(1.5)
        .fill(p.k.c.border_base)
        .a11y_hidden()
        .child(el().w(width * part).h(3.0).radius(1.5).fill(colour))
}

// ------------------------------------------------------------------ glyphs

macro_rules! glyph {
    ($(#[$doc:meta])* $name:ident, $body:literal) => {
        $(#[$doc])*
        pub(crate) static $name: Icon = Icon {
            name: concat!("look-", stringify!($name)),
            view: 24.0,
            body: $body,
        };
    };
}

glyph!(
    /// ✓ Done.
    CHECK,
    "<path d=\"M5 12.5l4.5 4.5L19 7.5\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
);
glyph!(
    /// ✗ Failed.
    CROSS,
    "<path d=\"M7 7l10 10M17 7L7 17\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.2\" stroke-linecap=\"round\"/>"
);
glyph!(
    /// ✦ The assistant thinking (and the words the person said).
    SPARK,
    "<path d=\"M12 3l1.9 5.6L19.5 10.5l-5.6 1.9L12 18l-1.9-5.6L4.5 10.5l5.6-1.9z\" fill=\"currentColor\"/>"
);
glyph!(
    /// ? Needs you.
    ASK,
    "<path d=\"M9.2 9a2.9 2.9 0 1 1 4.2 2.6c-.9.5-1.4 1.1-1.4 2.1v.6M12 17.6v.1\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.4\" stroke-linecap=\"round\"/>"
);
glyph!(
    /// Listening: three bars of a level.
    WAVE,
    "<path d=\"M6 10v4M10 7v10M14 5v14M18 9v6\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.2\" stroke-linecap=\"round\"/>"
);
glyph!(
    /// Working: the agent's pointer.
    POINTER,
    "<path d=\"M6 3.5l12 7-5.2 1.4L9.9 17z\" fill=\"currentColor\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linejoin=\"round\"/>"
);
glyph!(
    /// In front: a hand (the agent has the person's controls).
    HAND,
    "<path d=\"M8 12V6.5a1.5 1.5 0 0 1 3 0V11M11 10V5a1.5 1.5 0 0 1 3 0v6M14 10.5V7a1.5 1.5 0 0 1 3 0v7a6 6 0 0 1-6 6h-.5a6 6 0 0 1-4.6-2.2L4 15.5a1.5 1.5 0 0 1 2.3-1.9L8 15\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
);
glyph!(
    /// A parked window (a mini's caption).
    PARK,
    "<path d=\"M4 6h11v9H4zM15 9h3a2 2 0 0 1 2 2v7H9v-3\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linejoin=\"round\"/>"
);
glyph!(
    /// Pin a stage panel.
    PIN,
    "<path d=\"M9 4h6l-1 5 3 3H7l3-3zM12 12v8\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
);

// ------------------------------------------------------------------ states

/// The seven states a request goes through (§2), each with one chip word,
/// one glyph, and one colour, the same everywhere (island, rail, outline
/// chip, VoiceOver).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum State {
    Listening,
    Thinking,
    Working,
    /// The agent has the person's input or a window in front.
    InFront,
    NeedsYou,
    Done,
    Failed,
}

impl State {
    pub(crate) const ALL: [State; 7] = [
        State::Listening,
        State::Thinking,
        State::Working,
        State::InFront,
        State::NeedsYou,
        State::Done,
        State::Failed,
    ];

    /// The chip's word, in the person's language.
    pub(crate) fn word(self) -> &'static str {
        match self {
            State::Listening => tr("Listening"),
            State::Thinking => tr("Thinking"),
            State::Working => tr("Working"),
            State::InFront => tr("In front"),
            State::NeedsYou => tr("Needs you"),
            State::Done => tr("Done"),
            State::Failed => tr("Failed"),
        }
    }

    /// The word before translation.
    pub(crate) fn english(self) -> &'static str {
        match self {
            State::Listening => "Listening",
            State::Thinking => "Thinking",
            State::Working => "Working",
            State::InFront => "In front",
            State::NeedsYou => "Needs you",
            State::Done => "Done",
            State::Failed => "Failed",
        }
    }

    /// A stable name, for keys (`chip-needs-you`).
    pub(crate) fn key(self) -> &'static str {
        match self {
            State::Listening => "listening",
            State::Thinking => "thinking",
            State::Working => "working",
            State::InFront => "in-front",
            State::NeedsYou => "needs-you",
            State::Done => "done",
            State::Failed => "failed",
        }
    }

    pub(crate) fn glyph(self) -> &'static Icon {
        match self {
            State::Listening => &WAVE,
            State::Thinking => &SPARK,
            State::Working => &POINTER,
            State::InFront => &HAND,
            State::NeedsYou => &ASK,
            State::Done => &CHECK,
            State::Failed => &CROSS,
        }
    }

    /// The agent's hue while it listens, thinks, or works; the semantic
    /// colours after.
    pub(crate) fn colour(self, dark: bool) -> Color {
        match self {
            State::Listening | State::Thinking | State::Working | State::InFront => {
                palette::agent(dark)
            }
            State::NeedsYou => palette::needs(dark),
            State::Done => palette::done(dark),
            State::Failed => palette::failed(dark),
        }
    }

    /// A task's state as its chip says it: queued, running, held, and
    /// waiting out a limit all work (the row's line says which); `None`
    /// once it was stopped.
    pub(crate) fn of(phase: Phase) -> Option<State> {
        match phase {
            Phase::Queued | Phase::Running | Phase::Paused | Phase::Waiting { .. } => {
                Some(State::Working)
            }
            Phase::NeedsInput => Some(State::NeedsYou),
            Phase::Done => Some(State::Done),
            Phase::Failed => Some(State::Failed),
            Phase::Stopped => None,
        }
    }
}

/// `state`'s chip: its glyph and its word on a tint of its colour, keyed
/// `chip-<state>`.
pub(crate) fn chip(p: Paint, state: State) -> El {
    chip_words(p, state, state.word())
}

/// `state`'s chip with other words ("1 needs you"); the word itself is
/// still what a screen reader hears first.
pub(crate) fn chip_words(p: Paint, state: State, words: &str) -> El {
    let colour = state.colour(p.k.dark);
    let ink = palette::chip_ink(colour, p.k.dark);
    let spoken = if words == state.word() {
        words.to_owned()
    } else {
        format!("{}: {words}", state.word())
    };
    row()
        .key(format!("chip-{}", state.key()))
        .h(CHIP_H)
        .px(7.0)
        .gap(4.0)
        .center_y()
        .radius(CHIP_H / 2.0)
        .fill(p.k.alpha(colour, palette::CHIP_TINT))
        .role("status")
        .label(spoken)
        .child(icon(state.glyph(), 11.0).color(ink).a11y_hidden())
        .child(
            text(words.to_owned())
                .font(meta(p))
                .color(ink)
                .a11y_hidden(),
        )
}

// ------------------------------------------------------------------ buttons

/// What a pill button is to its card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// The one accent action of a card: inverse fill.
    Primary,
    /// Up to two more: a hairline ring.
    Secondary,
    /// A quiet one (Stop beside Take over): no ring.
    Quiet,
}

/// A pill button, [`BUTTON_H`] tall, its label in [`label`] type.
pub(crate) fn button<S: 'static>(
    p: Paint,
    kind: Kind,
    words: &str,
    on_press: impl Fn(&mut S) + 'static,
) -> El {
    let c = &p.k.c;
    let style = label(p);
    let words = words.to_owned();
    let shown = words.clone();
    // The primary is the kit's inverse ink as its fill in both
    // appearances (black on light cards, white on dark ones).
    let primary = kind == Kind::Primary;
    let (fill, ink) = (c.text_base, c.bg_base.alpha(1.0));
    let built = Button::new(words)
        .variant(match kind {
            Kind::Primary => ButtonVariant::Ghost,
            Kind::Secondary => ButtonVariant::Outline,
            Kind::Quiet => ButtonVariant::GhostMuted,
        })
        .control_size(ControlSize::Small)
        .label_with(move |_, kit_ink| {
            text(shown)
                .font(style)
                .color(if primary { ink } else { kit_ink })
        })
        .on_press(on_press)
        .build(p.k)
        .h(BUTTON_H)
        .px(12.0)
        .radius(BUTTON_H / 2.0);
    if primary { built.fill(fill) } else { built }
}

// ------------------------------------------------------------------ the card

/// One card in the anatomy above. [`Card::named`] sets the key prefix its
/// parts take: `<name>-head`, `<name>-title`, `<name>-hint`, `<name>-foot`.
#[derive(Default)]
pub(crate) struct Card {
    /// The key prefix of its parts.
    pub(crate) name: &'static str,
    /// The header's lead: a task's [`dot`], ✦, ✓ or ✗.
    pub(crate) lead: Option<El>,
    /// Who or what: the task's title, or the words the person said.
    pub(crate) title: String,
    /// The state, right of the title ([`chip`]).
    pub(crate) chip: Option<El>,
    /// The header's right end, past the chip: × and pin, on the stage only.
    pub(crate) trail: Option<El>,
    /// What the card says: one thing, at most four lines (more goes to
    /// the stage).
    pub(crate) body: Vec<El>,
    /// How to answer by voice ("Hold ⌘ and say it"), at the footer's left.
    pub(crate) hint: Option<String>,
    /// Up to two secondary actions, left of the primary.
    pub(crate) secondary: Vec<El>,
    /// The one primary action, at the footer's right end.
    pub(crate) primary: Option<El>,
    /// A footer of its own in place of the hint and the actions (a typed
    /// answer's field).
    pub(crate) foot: Option<El>,
    /// Its width; `None` fills its parent.
    pub(crate) width: Option<f32>,
}

impl Card {
    /// An empty card whose parts are keyed `<name>-…`.
    pub(crate) fn named(name: &'static str) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }

    /// How many actions it offers: (primary, secondary).
    pub(crate) fn actions(&self) -> (usize, usize) {
        (usize::from(self.primary.is_some()), self.secondary.len())
    }
}

/// The card: header (lead · title · chip · trail), body, footer (hint left,
/// actions right), in the card [`material`].
pub(crate) fn card(p: Paint, card: Card) -> El {
    debug_assert!(
        card.secondary.len() <= 2,
        "a card offers at most two secondary actions"
    );
    let name = card.name;
    let header = (card.lead.is_some()
        || !card.title.is_empty()
        || card.chip.is_some()
        || card.trail.is_some())
    .then(|| {
        row()
            .key(format!("{name}-head"))
            .w_full()
            .gap(8.0)
            .center_y()
            .children(card.lead)
            .child(
                text(card.title)
                    .key(format!("{name}-title"))
                    .font(label(p))
                    .color(p.muted())
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .children(card.chip)
            .children(card.trail)
    });
    let footer =
        match card.foot {
            Some(foot) => Some(foot),
            None => (card.hint.is_some() || card.primary.is_some() || !card.secondary.is_empty())
                .then(|| {
                    row()
                        .key(format!("{name}-foot"))
                        .w_full()
                        .mt(2.0)
                        .gap(8.0)
                        .center_y()
                        .child(el().grow(1.0).min_w(0.0).children(
                            card.hint.map(|words| {
                                hint(p, &words).key(format!("{name}-hint")).truncate()
                            }),
                        ))
                        .children(card.secondary)
                        .children(card.primary)
                }),
        };
    let body = col()
        .w_full()
        .padding(PADDING)
        .gap(GAP)
        .children(header)
        .children(card.body)
        .children(footer);
    let body = match card.width {
        Some(width) => body.w(width),
        None => body,
    };
    material(p, body)
}

/// A card's body text: 14 pt, wrapping.
pub(crate) fn body_text(p: Paint, words: &str) -> El {
    text(words.to_owned())
        .font(body(p))
        .color(p.text())
        .wrap()
        .w_full()
}

/// A voice hint or meta line, faint: "Hold ⌘ and say it".
pub(crate) fn hint(p: Paint, words: &str) -> El {
    text(words.to_owned()).font(meta(p)).color(p.faint())
}

/// Where a step stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    /// Passed: ✓ in green, the words muted.
    Done,
    /// On it now: the agent's dot, the words in full ink.
    Now,
    /// Still to come: an empty ring, the words faint.
    Next,
}

/// One step or reasoning line: its mark, an optional small label
/// ("Understood", "Plan", "Now", "Next"), and its words. Past and present
/// never look alike (✓ against a filled dot), and nothing spins.
pub(crate) fn step_line(p: Paint, tag: Option<&str>, words: &str, step: Step) -> El {
    let c = &p.k.c;
    let mark = match step {
        Step::Done => icon(&CHECK, 12.0).color(palette::done(p.k.dark)),
        Step::Now => el()
            .size(12.0, 12.0)
            .center()
            .child(dot(palette::agent(p.k.dark), 7.0)),
        Step::Next => el()
            .size(12.0, 12.0)
            .center()
            .child(el().size(7.0, 7.0).radius(3.5).border(1.2, c.border_strong)),
    };
    let ink = match step {
        Step::Done => p.muted(),
        Step::Now => p.text(),
        Step::Next => p.faint(),
    };
    row()
        .w_full()
        .gap(8.0)
        .items_start()
        .child(el().pt(4.0).child(mark.key("step-mark").a11y_hidden()))
        .children(tag.map(|tag| {
            text(tag.to_owned())
                .w(76.0)
                .font(meta(p))
                .color(p.faint())
                .pt(2.0)
        }))
        .child(
            text(words.to_owned())
                .key("step-text")
                .font(body(p))
                .color(ink)
                .wrap()
                .max_lines(2)
                .grow(1.0)
                .min_w(0.0),
        )
}

// ------------------------------------------------------------------ live minis

/// How often the rail looks again at its minis' streams while they show.
pub(crate) const LIVE_TICK: Duration = Duration::from_secs(1);
/// A stream with no frame for longer than this has stalled: its mini says
/// how long ago its last frame came instead of "Live".
pub(crate) const STALL: Duration = Duration::from_secs(3);

/// How live a mini's picture is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Liveness {
    /// Frames keep coming: a small pulsing "Live" dot.
    Live,
    /// The last frame came this long ago (more than [`STALL`]).
    Stalled(Duration),
    /// No picture yet: the page's or window's words stand in.
    Waiting,
    /// No picture can come (closed, minimized, no Screen Recording, a light
    /// browser that draws none).
    None,
}

/// A picture's [`Liveness`] from how long ago its latest frame came
/// (`None`: no frame yet) and whether it can't get one.
pub(crate) fn liveness(age: Option<Duration>, failed: bool) -> Liveness {
    match age {
        Some(age) if age > STALL => Liveness::Stalled(age),
        Some(_) => Liveness::Live,
        None if failed => Liveness::None,
        None => Liveness::Waiting,
    }
}

/// "4 s ago", "2 min ago".
pub(crate) fn ago(age: Duration) -> String {
    let seconds = age.as_secs();
    if seconds < 60 {
        tr_fill("{} s ago", &seconds.to_string())
    } else {
        tr_fill("{} min ago", &(seconds / 60).to_string())
    }
}

/// The badge in a mini's top-left corner: "● Live", the dot pulsing
/// (still with Reduce motion), while frames come; "8 s ago" once they
/// stall; nothing without a picture (its fallback says why). On a dark
/// glass pill so it reads over any picture.
pub(crate) fn live_badge(p: Paint, liveness: Liveness, still: bool) -> Option<El> {
    let (words, live) = match liveness {
        Liveness::Live => (tr("Live").to_owned(), true),
        Liveness::Stalled(age) => (ago(age), false),
        Liveness::Waiting | Liveness::None => return None,
    };
    let dot = dot(
        if live {
            palette::agent(true)
        } else {
            Color::WHITE.alpha(0.55)
        },
        6.0,
    );
    // While live, a soft halo breathes around the dot (still with Reduce
    // motion); the dot and the words stay steady.
    let halo = (live && !still).then(|| {
        el().absolute(-2.0, -2.0)
            .size(10.0, 10.0)
            .radius(5.0)
            .fill(palette::agent(true).alpha(0.45))
            .a11y_hidden()
            .looping(rune_motion::LoopSpec::opacity(1.2, 0.0, 0.0, 1.0))
    });
    let dot = el().size(6.0, 6.0).children(halo).child(dot);
    Some(
        row()
            .key(if live { "mini-live" } else { "mini-stalled" })
            .h(16.0)
            .px(6.0)
            .gap(4.0)
            .center_y()
            .radius(8.0)
            .fill(Color::BLACK.alpha(0.55))
            .label(words.clone())
            .child(dot)
            .child(
                text(words)
                    .font(TextStyle {
                        size: 10.0,
                        line_height: 12.0,
                        ..meta(p)
                    })
                    .color(Color::WHITE.alpha(if live { 0.95 } else { 0.75 }))
                    .a11y_hidden(),
            ),
    )
}

/// What a mini shows without a picture: its glyph, the page's title or the
/// window's app, and why there's no picture ("Waiting for a picture…", "No
/// live picture"), `w` × `h`.
pub(crate) fn no_picture(
    p: Paint,
    glyph: &'static Icon,
    title: &str,
    liveness: Liveness,
    (w, h): (f32, f32),
) -> El {
    let c = &p.k.c;
    let why = if liveness == Liveness::None {
        tr("No live picture")
    } else {
        tr("Waiting for a picture…")
    };
    col()
        .key("mini-none")
        .size(w, h)
        .padding(8.0)
        .gap(3.0)
        .radius(8.0)
        .fill(c.layer_02)
        .border_dash(4.0, 3.0)
        .border(p.hair.max(1.0), c.border_base)
        .child(icon(glyph, 13.0).color(p.faint()).a11y_hidden())
        .child(
            text(title.to_owned())
                .font(label(p))
                .color(p.muted())
                .wrap()
                .max_lines(2)
                .grow(1.0),
        )
        .child(
            text(why)
                .key("mini-why")
                .font(meta(p))
                .color(p.faint())
                .truncate(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_glyph_and_a_word() {
        let mut glyphs = Vec::new();
        let mut words = Vec::new();
        for state in State::ALL {
            assert!(!state.word().trim().is_empty(), "{state:?} has no word");
            assert!(!state.glyph().body.is_empty(), "{state:?} has no glyph");
            glyphs.push(state.glyph().name);
            words.push(state.english());
        }
        glyphs.sort_unstable();
        glyphs.dedup();
        words.sort_unstable();
        words.dedup();
        assert_eq!(glyphs.len(), State::ALL.len(), "each state its own glyph");
        assert_eq!(words.len(), State::ALL.len(), "each state its own word");
        // Every task phase but a stopped one shows as one of them.
        for phase in [
            Phase::Queued,
            Phase::Running,
            Phase::Paused,
            Phase::Waiting { until_ms: 0 },
            Phase::NeedsInput,
            Phase::Done,
            Phase::Failed,
        ] {
            assert!(State::of(phase).is_some(), "{phase:?}");
        }
        assert_eq!(State::of(Phase::Stopped), None);
    }

    #[test]
    fn a_mini_says_live_then_how_long_ago_once_its_stream_stalls() {
        use std::time::Duration;
        assert_eq!(liveness(Some(Duration::ZERO), false), Liveness::Live);
        assert_eq!(liveness(Some(STALL), false), Liveness::Live);
        let stalled = Duration::from_secs(8);
        assert_eq!(liveness(Some(stalled), false), Liveness::Stalled(stalled));
        assert_eq!(liveness(None, false), Liveness::Waiting);
        assert_eq!(liveness(None, true), Liveness::None);
        assert_eq!(ago(stalled), "8 s ago");
        assert_eq!(ago(Duration::from_secs(150)), "2 min ago");
    }

    #[test]
    fn the_type_ramp_has_four_sizes() {
        let p = Paint::new(false, 2.0);
        let sizes = [title(p), body(p), label(p), meta(p)].map(|style| style.size);
        assert_eq!(sizes, [17.0, 14.0, 12.0, 11.5]);
    }

    #[test]
    fn a_card_counts_its_actions() {
        let p = Paint::new(false, 2.0);
        let mut card = Card::named("t");
        assert_eq!(card.actions(), (0, 0));
        card.primary = Some(button::<()>(p, Kind::Primary, "Open", |_| {}));
        card.secondary = vec![button::<()>(p, Kind::Secondary, "More", |_| {})];
        assert_eq!(card.actions(), (1, 1));
    }
}
