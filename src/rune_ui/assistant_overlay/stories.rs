//! Labs › Assistant's overlays as playground stories (⇧⌘D, Prototypes ›
//! Assistant), fed by the made-up activity in `fixture` while the service
//! is a stub: the whole desktop playing the script, then the task stack,
//! the quick answer in each state, and the frame on their own.

use rune::{Cx, El, Playground, Stage, Story, el};

use super::super::controls::Paint;
use super::board::{Board, Bubble};
use super::desk::{self, Desk};
use super::{bubble, fixture, frame, stack};

/// Adds the assistant's stories to `playground`.
pub(crate) fn add<S: 'static>(playground: Playground<S>) -> Playground<S> {
    playground
        .story(desktop())
        .story(stack_story())
        .story(quick_answer())
        .story(frame_story())
}

/// The script plays on the pretend desktop: R replays it.
fn desktop() -> Story<Desk> {
    Story::new("Prototypes/Assistant/Desktop", Desk::empty, desk::view)
        .stage(Stage::Scroll)
        .poll(Desk::tick)
        .trigger('r', "Play", |desk, play| {
            *desk = Desk::empty();
            play.clear();
            for (at, event) in fixture::script() {
                play.after(at, move |desk: &mut Desk, _| desk.apply(&event));
            }
        })
        .trigger('f', "Agent mode on or off", |desk, _| {
            desk.frame = !desk.frame
        })
        .trigger('s', "Everything at once", |desk, play| {
            play.clear();
            *desk = Desk {
                breathe: true,
                ..Desk::settled()
            };
        })
}

/// A stack's state for its story: the board, and what was pressed.
struct StackStory {
    board: Board,
    note: String,
}

fn stack_story() -> Story<StackStory> {
    Story::new(
        "Prototypes/Assistant/Task stack",
        || StackStory {
            board: fixture::board(),
            note: String::new(),
        },
        stack_view,
    )
}

fn stack_view(state: &StackStory, cx: &mut Cx<StackStory>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let (w, h) = stack::SIZE;
    let note = (!state.note.is_empty()).then(|| {
        rune::text(state.note.clone())
            .key("note")
            .font(p.k.ty.small)
            .color(p.faint())
    });
    rune::col()
        .key("stack-story")
        .gap(8.0)
        .children(note)
        .child(
            el().key("stack-stage")
                .size(w, h)
                .padding(stack::INSET)
                .child(stack::plates(
                    p,
                    &state.board.stack(),
                    stack::Actions {
                        stop: |s: &mut StackStory, id| {
                            s.board.apply(
                                &crate::assistant::Event::Cancelled {
                                    conversation: id.into(),
                                },
                                None,
                                std::time::Instant::now(),
                            );
                            s.note = format!("Stopped {id}");
                        },
                        open: |s: &mut StackStory, id| {
                            s.note = format!("Would open “{id}” in Chats")
                        },
                    },
                )),
        )
}

/// The bubble's states, one variant each.
struct BubbleStory {
    bubble: Bubble,
}

fn quick_answer() -> Story<BubbleStory> {
    let at = |seconds: f64| {
        move |s: &mut BubbleStory| {
            if let Some(bubble) = fixture::bubble_at(seconds) {
                s.bubble = bubble;
            }
        }
    };
    Story::new(
        "Prototypes/Assistant/Quick answer",
        || BubbleStory {
            bubble: fixture::bubble_at(7.5).expect("the quick question has started"),
        },
        bubble_view,
    )
    .variant("thinking", at(7.5))
    .variant("working", at(9.1))
    .variant("streaming", at(11.0))
    .variant("answered", at(f64::MAX))
    .variant("long", |s| {
        s.bubble = fixture::bubble_ended(
            "What changed in the release notes?",
            Some(fixture::LONG_ANSWER),
            None,
        );
    })
    .variant("failed", |s| {
        s.bubble = fixture::bubble_ended(
            fixture::QUICK_QUESTION,
            None,
            Some("Claude Code isn't signed in. Open Terminal and run “claude” to sign in."),
        );
    })
}

fn bubble_view(state: &BubbleStory, cx: &mut Cx<BubbleStory>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    el().key("bubble-stage")
        .w(bubble::SIZE.0)
        .padding(bubble::INSET)
        .child(bubble::card(
            p,
            &state.bubble,
            bubble::Actions {
                dismiss: |_: &mut BubbleStory| {},
                open: |_: &mut BubbleStory, _| {},
            },
        ))
}

fn frame_story() -> Story<()> {
    Story::new(
        "Prototypes/Assistant/Frame",
        || (),
        |_: &(), cx: &mut Cx<()>| {
            let dark = cx.system_dark();
            let reduced = cx.system_reduced_motion();
            el().key("frame-stage")
                .w_full()
                .h_full()
                .child(frame::glow(dark, !reduced))
        },
    )
    .stage(Stage::Fill)
}
