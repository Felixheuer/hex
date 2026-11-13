//! The question a job waits on, answered right in the rail: its card (on
//! top, under the rail's head) says who asks (the task's dot and title,
//! "Needs you"), the question, and how to answer it, by voice ("Hold ⌘ and
//! say it") or typed: Reply (or a press on the card) opens a small field in
//! it; Return sends the answer to the job (`assistant::submit` in its
//! conversation, as Chats' composer would), Escape or × puts the field
//! away. Later puts the question behind the others (it still waits).
//!
//! The rail's panel takes the keyboard only while the field shows (the host
//! asks for it), and gives it back when the answer is sent or put away.

use rune::{El, TextEdit, col, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::board::Asking;
use super::look::{self, Card, Kind, State};
use super::{palette, stack};
use crate::desktop::i18n::{tr, tr_fill};

/// The card's width: the rail's.
pub(crate) const WIDTH: f32 = look::RAIL_W;
/// The room the card takes at most (its header, three lines, its footer);
/// [`FIELD_ROOM`] more while the field shows.
pub(crate) const ROOM: f32 = 160.0;
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
/// The question to answer now, on the one card anatomy (`look::card`): the
/// asking task's dot and title with the "Needs you" chip, the question (at
/// most three lines), and the footer: how to answer by voice at the left,
/// Later and Reply (the primary: type the answer) at the right; or the
/// field, while `reply` is its own. A press anywhere on it starts a typed
/// answer too.
pub(crate) fn question<S: 'static>(
    p: Paint,
    asking: &Asking,
    answer_key: &str,
    reply: Option<&Reply>,
    actions: stack::Actions<S>,
) -> El {
    let colour = palette::task(asking.colour, p.k.dark);
    let replying = reply.filter(|reply| reply.id == asking.id);
    let defer = actions.defer;
    let start = actions.reply.start;
    let (pressed, later, replies) = (asking.id.clone(), asking.id.clone(), asking.id.clone());
    let mut card = Card::named("question");
    card.lead = Some(super::project::with_dot(
        p,
        look::dot(colour, 9.0),
        asking.project.as_deref(),
    ));
    card.title = asking.title.clone();
    card.chip = Some(look::chip(p, State::NeedsYou));
    card.body = vec![
        look::body_text(p, &asking.question)
            .key("question-text")
            .max_lines(QUESTION_LINES)
            .font(look::body(p)),
    ];
    card.width = Some(WIDTH);
    match replying {
        Some(reply) => card.foot = Some(field(p, reply, colour, actions)),
        None => {
            card.hint = Some(tr_fill("Hold {} and say it", answer_key));
            card.secondary = vec![
                look::button(p, Kind::Secondary, tr("Later"), move |s: &mut S| {
                    defer(s, &later)
                })
                .key("later"),
            ];
            card.primary = Some(
                look::button(p, Kind::Primary, tr("Reply"), move |s: &mut S| {
                    start(s, &replies)
                })
                .key("reply"),
            );
        }
    }
    let bubble = look::card(p, card)
        .key(format!("question-{}", asking.id))
        .role("alert")
        .label(format!(
            "{}: {} · {}{}",
            State::NeedsYou.word(),
            asking.title,
            asking.question,
            asking
                .project
                .as_deref()
                .map(|name| format!(" · {}", tr_fill("Project: {}", name)))
                .unwrap_or_default()
        ))
        .live_region(rune::LiveRegion::Polite)
        .hit();
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
    let input = rune::text_input(&reply.edit, look::body(p))
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
