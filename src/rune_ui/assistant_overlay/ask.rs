//! The question a job waits on, answered right at the orb: its bubble (in
//! the job's colour, nearest the orb) says who asks, the question, and how
//! to answer it, by voice ("Hold ⌘ to answer") or typed: Reply (or a press
//! on the bubble) opens a small field in it; Return sends the answer to the
//! job (`assistant::submit` in its conversation, as Chats' composer would),
//! Escape or × puts the field away. Later puts the question behind the
//! others; × hides the bubble (the question still waits).
//!
//! The orb's panel takes the keyboard only while the field shows (the host
//! asks for it), and gives it back when the answer is sent or put away.

use rune::{El, TextEdit, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::board::Asking;
use super::{palette, stack};
use crate::desktop::i18n::{tr, tr_fill};

/// The bubble's width.
pub(crate) const WIDTH: f32 = 320.0;
/// The room the bubble takes at most (who asks, three lines, how to
/// answer); [`FIELD_ROOM`] more while the field shows.
pub(crate) const ROOM: f32 = 118.0;
pub(crate) const FIELD_ROOM: f32 = 64.0;
/// A question's most lines.
const QUESTION_LINES: usize = 3;
/// The typed answer's most lines before it scrolls.
const FIELD_LINES: usize = 3;

/// A typed answer under way: the question's job and what's typed.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Reply {
    pub(crate) id: String,
    pub(crate) edit: TextEdit,
}

impl Reply {
    /// A fresh field for `id`'s question, focused.
    pub(crate) fn new(id: &str) -> Self {
        let mut edit = TextEdit::default();
        edit.focused = true;
        Self {
            id: id.to_owned(),
            edit,
        }
    }

    /// What's typed, ready to send: `None` while it's blank.
    pub(crate) fn answer(&self) -> Option<String> {
        Some(self.edit.text.trim().to_owned()).filter(|text| !text.is_empty())
    }
}

impl std::fmt::Debug for Reply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reply")
            .field("id", &self.id)
            .field("text", &self.edit.text)
            .finish()
    }
}

/// What the field asks of the host.
pub(crate) struct ReplyActions<S> {
    /// Reply on a question (its job's id): open the field and take the
    /// keyboard.
    pub(crate) start: fn(&mut S, &str),
    /// Escape or ×: put the field away, giving the keyboard back.
    pub(crate) cancel: fn(&mut S),
    /// Return or Send: send what's typed to the job.
    pub(crate) send: fn(&mut S),
    /// Where the field's edits go.
    pub(crate) edit: fn(&mut S) -> &mut TextEdit,
}

impl<S> Clone for ReplyActions<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for ReplyActions<S> {}

/// The question to answer now, in its task's colour: who asks, the
/// question, how to answer it (or the field, while `reply` is its own),
/// Later, and × to put the bubble away (it still waits for the answer).
pub(crate) fn question<S: 'static>(
    p: Paint,
    asking: &Asking,
    answer_key: &str,
    reply: Option<&Reply>,
    actions: stack::Actions<S>,
) -> El {
    let c = &p.k.c;
    let colour = palette::task(asking.colour, p.k.dark);
    let replying = reply.filter(|reply| reply.id == asking.id);
    let dismiss = actions.dismiss;
    let defer = actions.defer;
    let start = actions.reply.start;
    let (pressed, put_away, later, replies) = (
        asking.id.clone(),
        asking.id.clone(),
        asking.id.clone(),
        asking.id.clone(),
    );
    let dot = el()
        .size(8.0, 8.0)
        .radius(4.0)
        .fill(colour.alpha(0.3))
        .border(1.5, colour)
        .a11y_hidden()
        .mt(5.0);
    let foot = match replying {
        Some(reply) => field(p, reply, colour, actions),
        None => row()
            .key("question-foot")
            .w_full()
            .gap(4.0)
            .center_y()
            .child(
                text(tr_fill("Hold {} to answer", answer_key))
                    .key("question-hint")
                    .truncate()
                    .font(p.k.ty.small)
                    .color(p.muted())
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                Button::new(tr("Reply"))
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut S| start(s, &replies))
                    .build(p.k)
                    .key("reply"),
            )
            .child(
                Button::new(tr("Later"))
                    .variant(ButtonVariant::GhostMuted)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut S| defer(s, &later))
                    .build(p.k)
                    .key("later"),
            ),
    };
    let bubble = row()
        .key(format!("question-{}", asking.id))
        .w(WIDTH)
        .pl(12.0)
        .pr(4.0)
        .py(8.0)
        .gap(8.0)
        .items_start()
        .radius(16.0)
        .fill(c.bg_base)
        .border(1.0, colour.alpha(0.7))
        .outline(3.0, 0.0, colour.alpha(0.16))
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 3.0,
            blur: 14.0,
            spread: 0.0,
            color: colour.alpha(if p.k.dark { 0.3 } else { 0.18 }),
            inset: false,
        })
        .role("alert")
        .live_region(rune::LiveRegion::Polite)
        .hit()
        .child(dot)
        .child(
            col()
                .key("question-lines")
                .gap(4.0)
                .grow(1.0)
                .min_w(0.0)
                .shrink()
                .child(
                    // Short enough that "asks" stays on the line.
                    text(tr_fill(
                        "{} asks",
                        &crate::assistant::store::shorten(&asking.title, 34),
                    ))
                    .key("question-who")
                    .truncate()
                    .font(p.k.ty.small_medium)
                    .color(colour),
                )
                .child(
                    text(asking.question.clone())
                        .key("question-text")
                        .wrap()
                        .max_lines(QUESTION_LINES)
                        .font(p.k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(foot),
        )
        .child(
            IconButton::new(p.k.glyph.close)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(tr("Hide question"))
                .on_press(move |s: &mut S| dismiss(s, &put_away))
                .build(p.k)
                .key("put-away"),
        );
    if replying.is_some() {
        bubble
    } else {
        // A press anywhere on it starts a typed answer.
        bubble.on_press(move |s: &mut S| start(s, &pressed))
    }
}

/// The typed answer: a field with Send, and how to send or put it away.
fn field<S: 'static>(
    p: Paint,
    reply: &Reply,
    colour: rune::Color,
    actions: stack::Actions<S>,
) -> El {
    let k = p.k;
    let ink = k.c.text_base;
    let send = actions.reply.send;
    let cancel = actions.reply.cancel;
    let empty = reply.answer().is_none();
    let input = rune::text_input(&reply.edit, k.ty.ui)
        .key("reply-input")
        .w_full()
        .color(ink)
        .placeholder(tr("Type your answer…"), k.c.text_faint)
        .caret(ink, k.selection())
        .label(tr("Your answer"))
        .on_edit(actions.reply.edit)
        .on_input_key(
            move |s: &mut S, event: &rune::KeyEvent, _tx: &mut rune::Tx| match event.key {
                rune::Key::Enter if !(event.mods.shift || event.mods.alt) => {
                    send(s);
                    true
                }
                rune::Key::Escape => {
                    cancel(s);
                    true
                }
                _ => false,
            },
        );
    let boxed = row()
        .key("reply-box")
        .w_full()
        .gap(4.0)
        .pl(8.0)
        .pr(3.0)
        .py(3.0)
        .items_end()
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, colour.alpha(0.6))
        .child(
            col()
                .key("reply-field")
                .role("textbox")
                .grow(1.0)
                .min_w(0.0)
                .py(3.0)
                .child(rune::auto_grow(input, 1, FIELD_LINES)),
        )
        .child(
            IconButton::new(&rune_theme_opencode::icons::ARROW_UP)
                .variant(IconButtonVariant::Submit)
                .control_size(ControlSize::Small)
                .label(tr("Send answer"))
                .disabled(empty)
                .on_press(move |s: &mut S| send(s))
                .build(k)
                .key("reply-send"),
        );
    col().key("reply").w_full().gap(3.0).child(boxed).child(
        text(tr("Return sends · Esc cancels"))
            .key("reply-hint")
            .truncate()
            .font(k.ty.small)
            .color(p.faint()),
    )
}

#[cfg(test)]
mod tests {
    use super::Reply;

    #[test]
    fn a_blank_answer_is_nothing_to_send() {
        let mut reply = Reply::new("invoices");
        assert!(reply.edit.focused, "the field takes the keys at once");
        assert_eq!(reply.answer(), None);
        reply.edit.text = "  \n ".into();
        assert_eq!(reply.answer(), None);
        reply.edit.text = " Finance \n".into();
        assert_eq!(reply.answer().as_deref(), Some("Finance"));
    }
}
