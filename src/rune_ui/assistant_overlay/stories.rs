//! The assistant's overlays as playground stories (⇧⌘D, Prototypes ›
//! Assistant), fed by the made-up activity in `fixture`: the whole desktop
//! playing the script, then the orb in each state (its job rows, a
//! question answered by typing, results to open or try again, the recall), the Here bubble in each
//! state, the Hands off pill, and the frame on their own.

use rune::{Cx, El, Playground, Stage, Story, el};

use super::super::controls::Paint;
use super::ask::Reply;
use super::board::{Board, Bubble};
use super::desk::{self, Desk};
use super::hint::Hint;
use super::listening::{self, Listening};
use super::{bubble, fixture, frame, hands, orb};

/// Adds the assistant's stories to `playground`.
pub(crate) fn add<S: 'static>(playground: Playground<S>) -> Playground<S> {
    playground
        .story(desktop())
        .story(orb_story())
        .story(quick_answer())
        .story(hands_story())
        .story(listening_story())
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
        .trigger('f', "Background work frame on or off", |desk, _| {
            desk.frame = !desk.frame
        })
        .trigger('n', "Tap+hold: a new background task", |desk, _| {
            desk.hint = match desk.hint {
                Some(Hint::Background) => None,
                _ => Some(Hint::Background),
            }
        })
        .trigger('h', "Hold: a Here question", |desk, _| {
            desk.hint = match desk.hint {
                Some(Hint::Here(_)) => None,
                _ => Some(Hint::Here("Mail".into())),
            }
        })
        .trigger('e', "Double-tap: the task list", |desk, _| {
            desk.expanded = !desk.expanded
        })
        .trigger('v', "Reading the answer aloud", |desk, _| {
            desk.speaking = !desk.speaking
        })
        .trigger('u', "Before setup", |desk, _| desk.setup = !desk.setup)
        .trigger('s', "Everything at once", |desk, play| {
            play.clear();
            *desk = Desk {
                breathe: true,
                ..Desk::settled()
            };
        })
}

/// The orb's state for its story: the board, whether its list is open, a
/// typed answer, the unfolded result, and what was pressed.
struct OrbStory {
    board: Board,
    open: bool,
    reply: Option<Reply>,
    open_result: Option<u64>,
    note: String,
}

fn orb_story() -> Story<OrbStory> {
    let with = |board: fn() -> Board, open: bool| {
        move |s: &mut OrbStory| {
            s.board = board();
            s.open = open;
            s.reply = None;
            s.open_result = None;
        }
    };
    Story::new(
        "Prototypes/Assistant/Orb",
        || OrbStory {
            board: fixture::board(),
            open: false,
            reply: None,
            open_result: None,
            note: String::new(),
        },
        orb_view,
    )
    .variant("bubbles", with(fixture::board, false))
    .variant("quiet", with(fixture::quiet_board, false))
    .variant("list", with(fixture::board, true))
    .variant("escalated", with(fixture::escalated_board, false))
    .variant("question", with(fixture::inbox_board, false))
    .variant("typing an answer", move |s| {
        with(fixture::inbox_board, false)(s);
        let mut reply = Reply::new("invoices");
        reply.edit.text = "Finance, please".into();
        s.reply = Some(reply);
    })
    .variant("many jobs", move |s| {
        with(|| fixture::many_jobs_board(9), false)(s);
    })
    .variant("team", move |s| with(|| fixture::team_board(5), false)(s))
    .variant("results", with(fixture::results_board, false))
    .variant("result unfolded", move |s| {
        with(fixture::results_board, false)(s);
        s.open_result = s
            .board
            .pops
            .items
            .iter()
            .find(|pop| pop.task == "suppliers")
            .map(|pop| pop.key);
    })
    .variant("recall", with(fixture::recall_board, false))
    .variant("recall list", with(fixture::recall_board, true))
}

fn orb_view(state: &OrbStory, cx: &mut Cx<OrbStory>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let (w, h) = orb::SIZE;
    let note = (!state.note.is_empty()).then(|| {
        rune::text(state.note.clone())
            .key("note")
            .font(p.k.ty.small)
            .color(p.faint())
    });
    let mut actions = orb::Actions::<OrbStory>::inert();
    let list = &mut actions.list;
    list.stop = |s, id| {
        s.board.apply(
            &crate::assistant::Event::Cancelled {
                conversation: id.into(),
            },
            None,
            std::time::Instant::now(),
        );
        s.note = format!("Stopped {id}");
    };
    list.take_over = |s, id| {
        s.board.apply(
            &crate::assistant::Event::Paused {
                conversation: id.into(),
            },
            None,
            std::time::Instant::now(),
        );
    };
    list.hand_back = |s, id| {
        s.board.apply(
            &crate::assistant::Event::Resumed {
                conversation: id.into(),
            },
            None,
            std::time::Instant::now(),
        );
    };
    list.stop_all = |s| s.note = "Would stop every task".into();
    list.toggle = |s| s.open = !s.open;
    list.dismiss = |s, id| {
        s.board.dismiss_question(id);
    };
    list.open_preview = |s, task, session| s.note = format!("Would open {session} of {task} live");
    list.defer = |s, id| s.note = format!("Would ask {id} later");
    list.reply.start = |s, id| s.reply = Some(Reply::new(id));
    list.reply.cancel = |s| s.reply = None;
    list.reply.send = |s| {
        if let Some(text) = s.reply.as_ref().and_then(Reply::answer) {
            s.note = format!("Would send “{text}”");
            s.reply = None;
        }
    };
    list.reply.edit = |s| &mut s.reply.get_or_insert_with(Reply::default).edit;
    list.results.open = |s, target| s.note = format!("Would open {target}");
    list.results.retry = |s, id| s.note = format!("Would try {id} again");
    list.results.dismiss = |s, key| {
        s.board.pops.dismiss(key);
    };
    list.results.expand = |s, key| s.open_result = (s.open_result != Some(key)).then_some(key);
    list.results.recall = |s, id| {
        s.open_result = s.board.recall(id, std::time::Instant::now());
        s.open = false;
    };
    actions.jobs.wall = |s, id| s.note = format!("Would show every picture of {id}");

    rune::col().key("orb-story").gap(8.0).children(note).child(
        el().key("orb-stage")
            .size(w, h)
            .padding(orb::INSET)
            .child(orb::content(
                p,
                &state.board,
                &orb::Look {
                    open: state.open,
                    still: cx.system_reduced_motion(),
                    answer_key: fixture::ANSWER_KEY.into(),
                    reply: state.reply.clone(),
                    open_result: state.open_result,
                    ..Default::default()
                },
                actions,
            )),
    )
}

/// The bubble's states, one variant each.
struct BubbleStory {
    bubble: Bubble,
    speaking: bool,
    setup: bool,
}

fn quick_answer() -> Story<BubbleStory> {
    let at = |seconds: f64| {
        move |s: &mut BubbleStory| {
            if let Some(bubble) = fixture::bubble_at(seconds) {
                s.bubble = bubble;
                s.speaking = false;
                s.setup = false;
            }
        }
    };
    let ended =
        |question: &'static str, answer: Option<&'static str>, error: Option<&'static str>| {
            move |s: &mut BubbleStory| {
                s.bubble = fixture::bubble_ended(question, answer, error);
                s.speaking = false;
                s.setup = false;
            }
        };
    Story::new(
        "Prototypes/Assistant/Quick answer",
        || BubbleStory {
            bubble: fixture::bubble_at(7.5).expect("the quick question has started"),
            speaking: false,
            setup: false,
        },
        bubble_view,
    )
    .variant("thinking", at(7.5))
    .variant("working", at(9.1))
    .variant("streaming", at(11.0))
    .variant("answered", at(f64::MAX))
    .variant("speaking", move |s| {
        at(f64::MAX)(s);
        s.speaking = true;
    })
    .variant(
        "long",
        ended(
            "What changed in the release notes?",
            Some(fixture::LONG_ANSWER),
            None,
        ),
    )
    .variant(
        "markdown",
        ended(
            "How do I free up disk space?",
            Some(fixture::MARKDOWN_ANSWER),
            None,
        ),
    )
    .variant(
        "failed",
        ended(
            fixture::QUICK_QUESTION,
            None,
            Some("Claude Code isn't signed in. Open Terminal and run “claude” to sign in."),
        ),
    )
    .variant("before setup", |s| s.setup = true)
}

fn bubble_view(state: &BubbleStory, cx: &mut Cx<BubbleStory>) -> El {
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let actions = bubble::Actions {
        dismiss: |_: &mut BubbleStory| {},
        open: |_: &mut BubbleStory, _| {},
        setup: |_: &mut BubbleStory| {},
    };
    let card = if state.setup {
        bubble::setup_card(p, actions)
    } else {
        bubble::card(
            p,
            &state.bubble,
            bubble::Voice {
                speaking: state.speaking,
                still: cx.system_reduced_motion(),
                level: None,
            },
            fixture::ANSWER_KEY,
            actions,
        )
    };
    el().key("bubble-stage")
        .w(bubble::SIZE.0)
        .padding(bubble::INSET)
        .child(card)
}

fn hands_story() -> Story<()> {
    Story::new(
        "Prototypes/Assistant/Hands off",
        || (),
        |_: &(), cx: &mut Cx<()>| {
            let p = Paint::new(cx.system_dark(), cx.env().scale);
            let (w, h) = hands::SIZE;
            el().key("hands-stage")
                .size(w, h)
                .center()
                .child(hands::pill(p))
        },
    )
}

/// Conversation mode's listening, counting down eight seconds: L listens
/// again.
fn listening_story() -> Story<Listening> {
    Story::new(
        "Prototypes/Assistant/Listening",
        || Listening::start(std::time::Instant::now(), 8.0),
        |state: &Listening, cx: &mut Cx<Listening>| {
            let p = Paint::new(cx.system_dark(), cx.env().scale);
            let (w, h) = listening::SIZE;
            el().key("listening-stage")
                .size(w, h)
                .center()
                .child(listening::drawn(p, state, cx.system_reduced_motion()))
        },
    )
    .poll(|state: &mut Listening, _, tx| {
        let changed = state.tick(std::time::Instant::now());
        if state.left > 0.0 {
            tx.wake_after(std::time::Duration::from_millis(80));
        }
        changed
    })
    .trigger('l', "Listen again", |state, _| {
        *state = Listening::start(std::time::Instant::now(), 8.0)
    })
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
