//! Made-up assistant activity for the stories, the render tests, and a
//! development run with `HEX_ASSISTANT_FAKE=1`: six Background tasks in
//! every state (running, taken over, waiting out a usage limit, queued,
//! done, failed), a Here question that thinks, takes the pointer for a
//! moment ("Hands off"), steps, and streams its answer, and where their
//! windows are.

use std::time::Instant;

use crate::assistant::{Event, Origin, TaskWindow};

use super::board::{Board, Bubble};
use super::when;

/// How long the fixture's waiting task waits for its usage limit.
pub(crate) const LIMIT_MINUTES: u64 = 23;

pub(crate) const QUICK: &str = "quick";
pub(crate) const QUICK_QUESTION: &str = "What's on my calendar this afternoon?";
pub(crate) const QUICK_ANSWER: &str = "You have two things this afternoon: the design review \
    with Maya at 2 pm, and the team sync at 4:30. Nothing after five, so your evening is free.";
pub(crate) const LONG_ANSWER: &str = "Here's what changed in the release notes since last \
    week. The onboarding flow now asks for the microphone before the first dictation, and \
    the model picker remembers the language you chose. Meetings recorded from Zoom keep \
    their chapter titles when exported, and History search now matches words inside \
    hyphenated phrases. Three crash fixes landed for older Intel Macs, including the one \
    that happened when waking from sleep with a Bluetooth headset connected. The settings \
    window opens faster, and the Labs page groups its experiments by what they change. \
    Finally, the updater now resumes interrupted downloads instead of starting over, which \
    should help on slow connections.";

pub(crate) const MARKDOWN_ANSWER: &str = "You're at **92%** of 494 GB. The quickest wins:\n\n\
    - Empty the Trash (**31 GB**)\n\
    - Delete old iOS backups in *Settings › General › Storage*\n\
    - Clear Xcode's `DerivedData` folder\n\n\
    Run `du -sh ~/Library/Caches` to see what the caches take.";

/// A window of the fixture's, by its id, and where it sits on a 1280 × 800
/// desktop (global points, y down).
/// A rect in points: x, y, width, height.
pub(crate) type Rect = (f64, f64, f64, f64);

pub(crate) const WINDOWS: [(&str, &str, Rect); 4] = [
    ("Safari", "safari", (64.0, 92.0, 560.0, 400.0)),
    ("Mail", "mail", (700.0, 250.0, 450.0, 300.0)),
    ("Calendar", "calendar", (250.0, 520.0, 430.0, 230.0)),
    ("Arc", "arc", (740.0, 600.0, 400.0, 160.0)),
];

pub(crate) fn window(id: &str) -> TaskWindow {
    let (app, id, _) = WINDOWS
        .iter()
        .find(|(_, window, _)| *window == id)
        .copied()
        .unwrap_or(("Safari", "safari", (0.0, 0.0, 0.0, 0.0)));
    TaskWindow {
        app: app.into(),
        window_id: Some(id.into()),
    }
}

/// Where a fixture window is, scaled from the 1280 × 800 desktop onto one
/// `size` big (points).
pub(crate) fn frame(window: &TaskWindow, size: (f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let id = window.window_id.as_deref()?;
    let (_, _, (x, y, w, h)) = WINDOWS.iter().find(|(_, each, _)| *each == id)?;
    let (sx, sy) = (size.0 / 1280.0, size.1 / 800.0);
    Some((x * sx, y * sy, w * sx, h * sy))
}

fn started(id: &str, origin: Origin, question: &str) -> Event {
    Event::Started {
        conversation: id.into(),
        origin,
        question: question.into(),
    }
}

fn step(id: &str, description: &str) -> Event {
    Event::Step {
        conversation: id.into(),
        description: description.into(),
    }
}

fn opened(id: &str, window_id: &str) -> Event {
    Event::Window {
        conversation: id.into(),
        window: window(window_id),
    }
}

/// The whole show, as `(seconds from the start, event)`: Background work
/// starts, six tasks start, step, open windows, queue, fail, finish, get
/// taken over, and wait out a usage limit, and a Here question takes the
/// pointer for a moment, then is answered.
pub(crate) fn script() -> Vec<(f64, Event)> {
    let agent = Origin::Background;
    let mut script: Vec<(f64, Event)> = vec![
        (0.0, Event::ModeChanged { on: true }),
        (
            0.4,
            started("table", agent, "Find a table for two at Nopa tonight"),
        ),
        (
            0.7,
            Event::Thinking {
                conversation: "table".into(),
            },
        ),
        (1.3, step("table", "Opened Safari")),
        (1.4, opened("table", "safari")),
        (
            1.7,
            started(
                "mail",
                agent,
                "Draft replies to the three unread emails from Lena",
            ),
        ),
        (2.2, step("mail", "Opened Mail")),
        (2.3, opened("mail", "mail")),
        (
            2.6,
            Event::Queued {
                conversation: "rename".into(),
                origin: agent,
                question: "Rename the screenshots on the Desktop by date".into(),
            },
        ),
        (
            3.0,
            started(
                "dentist",
                agent,
                "Add Friday's dentist appointment to my calendar",
            ),
        ),
        (3.4, step("dentist", "Opened Calendar")),
        (3.5, opened("dentist", "calendar")),
        (
            3.8,
            started(
                "export",
                agent,
                "Export the Q3 numbers from Numbers as a PDF",
            ),
        ),
        (
            4.6,
            Event::Failed {
                conversation: "export".into(),
                message: "Numbers isn't installed on this Mac.".into(),
            },
        ),
        (4.4, step("table", "Typing “Nopa” in Resy's search field")),
        (5.0, step("dentist", "Clicked New Event")),
        (
            5.6,
            step("mail", "Writing a reply to “Venue for the offsite?”"),
        ),
        (
            6.6,
            Event::Answer {
                conversation: "dentist".into(),
                origin: agent,
                text: "Added “Dentist” on Friday at 9:30 to your Home calendar.".into(),
            },
        ),
        (7.0, step("table", "Chose 8:00 pm for 2 people")),
        (
            4.0,
            started(
                "news",
                agent,
                "Summarise today's top stories on Hacker News",
            ),
        ),
        (4.3, step("news", "Opened https://news.ycombinator.com")),
        (4.4, opened("news", "arc")),
        (4.9, step("news", "Read the top 10 stories")),
        (
            5.4,
            Event::Waiting {
                conversation: "news".into(),
                until_ms: when::now_ms() + LIMIT_MINUTES * 60_000,
                reason: "Claude's usage limit".into(),
            },
        ),
        (
            6.2,
            Event::Paused {
                conversation: "mail".into(),
            },
        ),
        (8.5, opened(QUICK, "calendar")),
        (
            8.6,
            Event::HandsOff {
                conversation: QUICK.into(),
                on: true,
            },
        ),
        (
            9.3,
            Event::HandsOff {
                conversation: QUICK.into(),
                on: false,
            },
        ),
        (7.4, started(QUICK, Origin::ActionShortcut, QUICK_QUESTION)),
        (
            7.6,
            Event::Thinking {
                conversation: QUICK.into(),
            },
        ),
        (8.4, step(QUICK, "Opened Calendar")),
        (9.0, step(QUICK, "Read this afternoon's events")),
    ];
    let mut at = 9.6;
    for word in QUICK_ANSWER.split_inclusive(' ') {
        script.push((
            at,
            Event::AnswerDelta {
                conversation: QUICK.into(),
                text: word.into(),
            },
        ));
        at += 0.07;
    }
    script.push((
        at + 0.2,
        Event::Answer {
            conversation: QUICK.into(),
            origin: Origin::ActionShortcut,
            text: QUICK_ANSWER.into(),
        },
    ));
    script.sort_by(|a, b| a.0.total_cmp(&b.0));
    script
}

/// The script, then the rest of the story for a development run: the mail
/// task handed back, every task finishing, and the Background work ending.
pub(crate) fn script_to_the_end() -> Vec<(f64, Event)> {
    let mut script = script();
    let end = script.last().map_or(0.0, |(at, _)| *at);
    script.push((
        end + 3.0,
        Event::Resumed {
            conversation: "mail".into(),
        },
    ));
    for (id, at) in [("table", 6.0), ("mail", 9.0), ("news", 13.0)] {
        script.push((
            end + at,
            Event::Answer {
                conversation: id.into(),
                origin: Origin::Background,
                text: "Done.".into(),
            },
        ));
    }
    script.push((
        end + 11.0,
        Event::Cancelled {
            conversation: "rename".into(),
        },
    ));
    script.push((end + 18.0, Event::ModeChanged { on: false }));
    script
}

/// The board after the whole script, every task still listed: one running,
/// one taken over, one queued, one done, one failed, one waiting.
pub(crate) fn board() -> Board {
    board_at(f64::MAX)
}

/// The board after the script's events up to `seconds`.
pub(crate) fn board_at(seconds: f64) -> Board {
    let now = Instant::now();
    let mut board = Board::default();
    for (_, event) in script().into_iter().filter(|(at, _)| *at <= seconds) {
        board.apply(&event, None, now);
    }
    board
}

/// The orb with nothing to say: the live tasks only, no bubbles.
pub(crate) fn quiet_board() -> Board {
    let mut board = board();
    board.tasks.retain(|task| task.phase.live());
    board.pops = Default::default();
    board
}

/// The quick question's bubble as it is after the script's events up to

/// `seconds`.
pub(crate) fn bubble_at(seconds: f64) -> Option<Bubble> {
    let now = Instant::now();
    let mut bubble = None;
    for (_, event) in script().into_iter().filter(|(at, _)| *at <= seconds) {
        Bubble::apply(&mut bubble, &event, now);
    }
    bubble
}

/// A quick question answered with `answer`, or failed with `error`.
pub(crate) fn bubble_ended(question: &str, answer: Option<&str>, error: Option<&str>) -> Bubble {
    let now = Instant::now();
    let mut bubble = Some(Bubble::new(QUICK.into(), question));
    let end = match (answer, error) {
        (_, Some(message)) => Event::Failed {
            conversation: QUICK.into(),
            message: message.into(),
        },
        (answer, None) => Event::Answer {
            conversation: QUICK.into(),
            origin: Origin::ActionShortcut,
            text: answer.unwrap_or_default().into(),
        },
    };
    Bubble::apply(&mut bubble, &end, now);
    bubble.expect("a bubble")
}
