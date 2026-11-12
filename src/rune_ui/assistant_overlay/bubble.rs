//! The quick question's bubble, just past the dictation HUD where Live
//! text and Failure notices show: the question as heard, "Thinking…", the
//! latest steps, then the answer as it streams. A long answer is cut with
//! "…" and the footer's "Open in Chats" shows the rest. It stays while the
//! answer is read aloud, then a few seconds more (`board::Bubble::due`);
//! a press, or Esc, dismisses it.

use rune::{Cx, El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Spinner};

use super::super::controls::Paint;
use super::board::{Bubble, BubblePhase};

/// The overlay: a readable measure, and room for about ten lines.
pub(crate) const SIZE: (f32, f32) = (440.0, 340.0);
/// The answer's most lines before "…".
const ANSWER_LINES: usize = 10;
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
    /// Whether it shows; a hidden bubble keeps its last card for the fade.
    pub(crate) shown: bool,
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
    root.children(
        state
            .bubble
            .as_ref()
            .map(|bubble| card(p, bubble, state.actions)),
    )
}

/// The card for `bubble`, as wide as its parent allows.
pub(crate) fn card<S: 'static>(p: Paint, bubble: &Bubble, actions: Actions<S>) -> El {
    let c = &p.k.c;
    let question = text(bubble.question.clone())
        .key("question")
        .truncate()
        .font(p.k.ty.small)
        .color(p.faint());
    let body: Vec<El> = match bubble.phase {
        BubblePhase::Thinking => vec![working(p, "Thinking…", "thinking")],
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
            text(bubble.answer.trim().to_owned())
                .key("answer")
                .wrap()
                .max_lines(ANSWER_LINES)
                .font(p.k.ty.ui)
                .color(p.text())
                .selectable(false),
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
            Button::new("Open in Chats")
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
        .border(p.hair, c.border_base)
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
        .children(footer)
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
