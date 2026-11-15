use std::time::{Duration, Instant};

use super::*;
use crate::assistant::{PlanStep, TaskState, TaskWindow};

fn id(n: u32) -> ConversationId {
    format!("c{n}")
}

fn started(n: u32, origin: Origin, question: &str) -> Event {
    Event::Started {
        conversation: id(n),
        origin,
        question: question.into(),
    }
}

fn step(n: u32, description: &str) -> Event {
    Event::Step {
        conversation: id(n),
        description: description.into(),
    }
}

fn window(app: &str) -> TaskWindow {
    TaskWindow {
        app: app.into(),
        window_id: None,
    }
}

#[test]
fn a_task_runs_its_steps_and_finishes_with_its_answer() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    assert!(stream.apply(
        &Event::Queued {
            conversation: id(1),
            origin: Origin::Background,
            question: "Book a table for two tonight".into(),
        },
        Some(3),
        t0,
    ));
    assert_eq!(stream.tasks[0].phase, Phase::Queued);
    assert_eq!(stream.tasks[0].colour, 3);
    stream.apply(&started(1, Origin::Background, "Book a table"), None, t0);
    assert_eq!(
        stream.tasks.len(),
        1,
        "Started after Queued is the same task"
    );
    assert_eq!(stream.tasks[0].phase, Phase::Running);
    assert_eq!(stream.tasks[0].colour, 3, "keeps its colour");
    assert_eq!(stream.tasks[0].activity, Activity::Thinking);
    stream.apply(&step(1, "Opened Safari"), None, t0);
    assert_eq!(stream.tasks[0].activity, Activity::Working);
    assert_eq!(stream.tasks[0].detail().as_deref(), Some("Opened Safari"));
    let safari = Event::Window {
        conversation: id(1),
        window: window("Safari"),
    };
    assert!(stream.apply(&safari, None, t0));
    assert!(!stream.apply(&safari, None, t0), "the same window once");
    assert_eq!(
        stream.tasks[0].log,
        [
            Logged::Step("Opened Safari".into()),
            Logged::Window("Safari".into())
        ],
        "its log says the window once too"
    );
    assert!(
        !stream.apply(
            &Event::AnswerDelta {
                conversation: id(1),
                text: "Booked".into(),
            },
            None,
            t0,
        ),
        "streamed text isn't read at a glance"
    );
    assert_eq!(stream.tasks[0].activity, Activity::Answering);
    assert_eq!(stream.tasks[0].answer, "Booked");
    stream.apply(
        &Event::Answer {
            conversation: id(1),
            origin: Origin::Background,
            text: "\nBooked Nopa for 8 pm.\nConfirmation sent to your email.".into(),
        },
        None,
        t0 + Duration::from_secs(10),
    );
    let task = &stream.tasks[0];
    assert_eq!(task.phase, Phase::Done);
    assert_eq!(task.detail().as_deref(), Some("Booked Nopa for 8 pm…"));
    assert!(stream.live_task(&id(1)).is_none());
    let leaves = t0 + Duration::from_secs(10) + FINISHED_LINGER;
    assert_eq!(stream.next_expiry(), Some(leaves));
    assert!(!stream.expire(leaves - Duration::from_millis(1)));
    assert!(stream.expire(leaves));
    assert!(stream.tasks.is_empty());
}

#[test]
fn a_stopped_task_leaves_sooner_and_late_events_change_nothing() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::Background, "Tidy my desktop"), None, t0);
    stream.apply(
        &Event::Cancelled {
            conversation: id(1),
        },
        None,
        t0,
    );
    assert_eq!(stream.tasks[0].phase, Phase::Stopped);
    assert!(!stream.apply(&step(1, "Moved a file"), None, t0));
    assert_eq!(stream.next_expiry(), Some(t0 + STOPPED_LINGER));
    assert!(!stream.apply(
        &Event::Thinking {
            conversation: id(9)
        },
        None,
        t0
    ));
}

#[test]
fn a_follow_up_starts_afresh_and_counts_its_time_again() {
    let t0 = Instant::now();
    let later = t0 + Duration::from_secs(60);
    let mut stream = Stream::default();
    stream.apply(
        &started(1, Origin::Background, "Compare suppliers"),
        None,
        t0,
    );
    stream.apply(&step(1, "Read the first offer"), None, t0);
    stream.apply(
        &Event::AnswerDelta {
            conversation: id(1),
            text: "So far".into(),
        },
        None,
        t0,
    );
    // Steering while it runs: its run goes on, the text in flight is
    // stored, so it starts afresh; it's been running since t0.
    stream.apply(
        &started(1, Origin::Background, "And the prices?"),
        None,
        later,
    );
    let task = &stream.tasks[0];
    assert!(task.answer.is_empty() && task.log.is_empty());
    assert_eq!(
        task.steps,
        ["Read the first offer"],
        "its row keeps its step"
    );
    assert_eq!(task.since, Some(t0));
    // Asked again after it ended (Try again): its time starts over.
    stream.apply(
        &Event::Failed {
            conversation: id(1),
            message: "The shop refused the login.".into(),
        },
        None,
        later,
    );
    stream.apply(
        &Event::Queued {
            conversation: id(1),
            origin: Origin::Background,
            question: "Please try again".into(),
        },
        None,
        later,
    );
    let task = &stream.tasks[0];
    assert_eq!(task.phase, Phase::Queued);
    assert_eq!(task.since, Some(later));
    assert!(task.error.is_none());
}

#[test]
fn a_failure_is_kept_even_before_its_task_was_seen() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    assert!(stream.apply(
        &Event::Failed {
            conversation: id(1),
            message: "No agent is set up".into(),
        },
        None,
        t0,
    ));
    let task = stream.task(&id(1)).unwrap();
    assert_eq!(task.phase, Phase::Failed);
    assert_eq!(task.error.as_deref(), Some("No agent is set up"));
    assert!(stream.stack().is_empty(), "no job of its own");
    stream.clear_failure(&id(1));
    assert!(stream.tasks.is_empty());
    stream.apply(&started(2, Origin::Chat, "a"), None, t0);
    stream.clear_failure(&id(2));
    assert!(stream.live_task(&id(2)).is_some(), "a live task stays");
}

#[test]
fn tasks_without_a_colour_take_the_first_free_one() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::Background, "a"), Some(0), t0);
    stream.apply(&started(2, Origin::Background, "b"), None, t0);
    stream.apply(&started(3, Origin::Background, "c"), None, t0);
    let colours: Vec<_> = stream.tasks.iter().map(|t| t.colour).collect();
    assert_eq!(colours, [0, 1, 2]);
    stream.apply(
        &Event::Cancelled {
            conversation: id(2),
        },
        None,
        t0,
    );
    stream.apply(&started(4, Origin::Background, "d"), None, t0);
    assert_eq!(
        stream.tasks[3].colour, 1,
        "a finished task's colour is free again"
    );
    stream.apply(&started(5, Origin::Background, "e"), Some(11), t0);
    assert_eq!(stream.tasks[4].colour, 11 % COLOURS);
}

#[test]
fn holds_say_why_a_task_waits() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::Background, "Summarise"), None, t0);
    assert_eq!(stream.hold(&id(1)), None);
    stream.apply(
        &Event::Waiting {
            conversation: id(1),
            until_ms: 1_700_000_000_000,
            reason: "Claude's usage limit".into(),
        },
        None,
        t0,
    );
    assert!(stream.waiting());
    assert_eq!(
        stream.hold(&id(1)),
        Some(Hold::Waiting {
            until_ms: 1_700_000_000_000,
            reason: "Claude's usage limit".into()
        })
    );
    assert!(stream.tasks[0].phase.live(), "still stoppable");
    stream.apply(
        &Event::Thinking {
            conversation: id(1),
        },
        None,
        t0,
    );
    assert_eq!(stream.tasks[0].phase, Phase::Running);
    assert!(!stream.waiting());
    assert_eq!(stream.hold(&id(1)), None);

    stream.apply(
        &Event::Paused {
            conversation: id(1),
        },
        None,
        t0,
    );
    stream.apply(&step(1, "Looked again"), None, t0);
    assert_eq!(
        stream.hold(&id(1)),
        Some(Hold::Paused),
        "only Resumed ends it"
    );
    stream.apply(
        &Event::Resumed {
            conversation: id(1),
        },
        None,
        t0,
    );
    assert_eq!(stream.hold(&id(1)), None);

    stream.apply(
        &Event::Question {
            conversation: id(1),
            text: " Which account? ".into(),
        },
        None,
        t0,
    );
    assert_eq!(
        stream.hold(&id(1)),
        Some(Hold::Asking("Which account?".into()))
    );
    assert_eq!(stream.answered(&id(1)).as_deref(), Some("Which account?"));
    assert_eq!(stream.hold(&id(1)), None, "answered: its box goes");
    assert_eq!(Hold::of(TaskState::Paused), Some(Hold::Paused));
    assert_eq!(Hold::of(TaskState::Running), None);
}

#[test]
fn hands_off_follows_its_task_and_ends_with_it() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::ActionShortcut, "Weather?"), None, t0);
    assert!(stream.apply(
        &Event::HandsOff {
            conversation: id(1),
            on: true
        },
        None,
        t0
    ));
    assert_eq!(stream.hands_off().map(|t| t.id.clone()), Some(id(1)));
    assert!(
        stream.frame_until(t0).is_some(),
        "the frame shows meanwhile"
    );
    stream.apply(
        &Event::Answer {
            conversation: id(1),
            origin: Origin::ActionShortcut,
            text: "Done".into(),
        },
        None,
        t0,
    );
    assert!(stream.hands_off().is_none(), "a finished task lets go");
    assert!(stream.frame_until(t0).is_none());
}

#[test]
fn questions_wait_newest_first_and_plans_count_their_steps() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    for n in 1..=2 {
        stream.apply(&started(n, Origin::Background, "a"), None, t0);
        stream.apply(
            &Event::AnswerDelta {
                conversation: id(n),
                text: "Looking".into(),
            },
            None,
            t0,
        );
        stream.apply(
            &Event::Question {
                conversation: id(n),
                text: format!("Question {n}? "),
            },
            None,
            t0,
        );
    }
    assert_eq!(stream.tasks[0].phase, Phase::NeedsInput);
    assert!(
        stream.tasks[0].answer.is_empty(),
        "what it said is stored with the question"
    );
    let asking: Vec<_> = stream.questions().iter().map(|t| t.id.clone()).collect();
    assert_eq!(asking, [id(2), id(1)], "newest first");
    assert_eq!(stream.open_questions(), 2);
    stream.apply(&step(1, "Still looking"), None, t0);
    assert_eq!(stream.tasks[0].phase, Phase::NeedsInput, "until answered");
    // The answer arrives as a follow-up in the same conversation.
    stream.apply(&started(1, Origin::Background, "Finance"), None, t0);
    assert_eq!(stream.tasks[0].phase, Phase::Running);
    assert!(stream.tasks[0].question.is_none());

    let plan_step = |title: &str, done| PlanStep {
        title: title.into(),
        done,
    };
    stream.apply(
        &Event::Plan {
            conversation: id(1),
            steps: vec![
                plan_step("a", true),
                plan_step("b", false),
                plan_step("c", false),
            ],
        },
        None,
        t0,
    );
    assert_eq!(stream.tasks[0].progress(), Some((1, 3)));
    assert_eq!(stream.tasks[0].current_step(), Some(1));

    stream.apply(
        &Event::QuestionsChanged {
            active: None,
            waiting: 3,
        },
        None,
        t0,
    );
    assert_eq!(stream.open_questions(), 3, "the inbox, once told, counts");
    assert_eq!(stream.active_id(), None);
}

#[test]
fn an_escalated_quick_turn_becomes_a_job() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::ActionShortcut, "Fix this"), None, t0);
    assert!(stream.stack().is_empty());
    assert!(stream.apply(
        &Event::Escalated {
            conversation: id(1)
        },
        None,
        t0
    ));
    assert_eq!(stream.stack().len(), 1);
    assert!(stream.tasks[0].escalated);
    assert!(!stream.apply(
        &Event::Escalated {
            conversation: id(1)
        },
        None,
        t0
    ));
}

#[test]
fn listed_tasks_are_adopted_with_their_states() {
    let status = |n: u32, state| TaskStatus {
        conversation: id(n),
        origin: Origin::Background,
        title: "t".into(),
        state,
        step: Some("Opened Mail".into()),
        windows: Vec::new(),
        colour: 9,
    };
    let mut stream = Stream::seeded(&[
        status(1, TaskState::Paused),
        status(2, TaskState::Waiting { until_ms: 5 }),
    ]);
    assert_eq!(stream.tasks[0].phase, Phase::Paused);
    assert_eq!(stream.tasks[1].phase, Phase::Waiting { until_ms: 5 });
    assert_eq!(
        stream.tasks[0].colour,
        9 % crate::assistant::view::COLOURS,
        "wraps"
    );
    assert_eq!(stream.tasks[0].steps, ["Opened Mail"]);
    assert_eq!(stream.tasks[0].activity, Activity::Working);
    stream.apply(
        &Event::Resumed {
            conversation: id(1),
        },
        None,
        Instant::now(),
    );
    assert!(
        !stream.adopt(&[status(1, TaskState::Paused), status(2, TaskState::Running)]),
        "known tasks keep what their events said"
    );
    assert_eq!(stream.tasks[0].phase, Phase::Running);
    assert!(stream.adopt(&[status(3, TaskState::Queued)]));
    assert_eq!(stream.tasks.len(), 3);
}

#[test]
fn a_task_keeps_its_last_three_steps_its_log_and_knows_where_it_works() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::Background, "a"), None, t0);
    assert_eq!(stream.tasks[0].place(), Place::Unknown);
    let steps = [
        "Searched the web for Nopa",
        "Read the menu",
        "Read the menu",
        "Chose 8 pm",
        "Booked",
    ];
    for description in steps {
        stream.apply(&step(1, description), None, t0);
    }
    assert_eq!(
        stream.tasks[0].steps,
        ["Read the menu", "Chose 8 pm", "Booked"]
    );
    assert_eq!(stream.tasks[0].log.len(), steps.len(), "the log keeps all");
    for n in 0..LOG_MAX {
        stream.apply(&step(1, &format!("Step {n}")), None, t0);
    }
    assert_eq!(stream.tasks[0].log.len(), LOG_MAX, "up to its cap");
    stream.apply(&started(2, Origin::Background, "b"), None, t0);
    stream.apply(&step(2, "Opened https://resy.com"), None, t0);
    assert_eq!(stream.tasks[1].place(), Place::Web);
    stream.apply(
        &Event::Window {
            conversation: id(2),
            window: window("Safari"),
        },
        None,
        t0,
    );
    assert_eq!(stream.tasks[1].place(), Place::Web, "a browser's window");
    stream.apply(
        &Event::Window {
            conversation: id(2),
            window: window("Numbers"),
        },
        None,
        t0,
    );
    assert_eq!(stream.tasks[1].place(), Place::Desktop);
}

#[test]
fn the_understood_line_is_kept_until_a_follow_up_starts_afresh() {
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(
        &started(1, Origin::ActionShortcut, "Book Nopa for two at eight"),
        None,
        t0,
    );
    assert!(stream.apply(
        &Event::Understood {
            conversation: id(1),
            text: " Nopa, 2 people, tonight around 8 pm ".into(),
        },
        None,
        t0,
    ));
    assert_eq!(
        stream.tasks[0].understood.as_deref(),
        Some("Nopa, 2 people, tonight around 8 pm")
    );
    stream.apply(
        &started(1, Origin::ActionShortcut, "Make it 8:15"),
        None,
        t0,
    );
    assert_eq!(stream.tasks[0].understood, None);
}

fn tool(n: u32, name: &str, input: serde_json::Value) -> Event {
    let shown = ToolUse::of(name, &input);
    Event::Tool {
        conversation: id(n),
        lines: console::call_lines(shown.kind, &input),
        tool: shown,
    }
}

fn said(n: u32, text: &str) -> Event {
    Event::AnswerDelta {
        conversation: id(n),
        text: text.into(),
    }
}

#[test]
fn a_task_without_a_plan_says_what_it_does_now() {
    crate::desktop::i18n::apply(Some("de"));
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(
        &started(1, Origin::Background, "Verbessere dich iterativ"),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0),
        None,
        "nothing yet: it thinks"
    );
    // What it said, while nothing runs: its first sentence.
    stream.apply(
        &said(
            1,
            "Ich prüfe noch die native Kontakte-Abfrage. Danach teste ich.",
        ),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Ich prüfe noch die native Kontakte-Abfrage.")
    );
    // A tool now: its kind and description, in the person's language.
    let search = tool(
        1,
        "Grep",
        serde_json::json!({"pattern": "link_contact", "description": "Kontakte verknüpfen"}),
    );
    assert!(stream.apply(&search, None, t0));
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Sucht im Code: Kontakte verknüpfen")
    );
    stream.apply(
        &tool(
            1,
            "Read",
            serde_json::json!({"file_path": "/repo/src/setup_gaps.rs"}),
        ),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Liest setup_gaps.rs")
    );
    // A command never shows itself, only its description or its kind.
    stream.apply(
        &tool(
            1,
            "Bash",
            serde_json::json!({"command": "TOKEN=abc123456 ./deploy.sh"}),
        ),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Führt Befehl aus")
    );
    assert_eq!(
        stream.tasks[0].console.len(),
        3,
        "search, read, and command lines"
    );
    assert!(
        stream.tasks[0]
            .console
            .lines()
            .all(|line| !line.text.contains("abc123456")),
        "the terminal keeps no secret"
    );
    stream.apply(
        &tool(
            1,
            "WebFetch",
            serde_json::json!({"url": "https://www.example.com/a"}),
        ),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Lädt example.com")
    );
    stream.apply(
        &tool(
            1,
            "mcp__agent-browser__browser_open",
            serde_json::json!({"url": "https://example.com"}),
        ),
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Öffnet Browser: example.com")
    );
    // Its end: back to what it said last, before the tool.
    stream.apply(
        &Event::ToolDone {
            conversation: id(1),
            lines: Vec::new(),
        },
        None,
        t0,
    );
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Ich prüfe noch die native Kontakte-Abfrage.")
    );
    // What it says after a tool starts afresh.
    stream.apply(&search, None, t0);
    stream.apply(&said(1, "Gefunden: zwei Stellen\n\n"), None, t0);
    assert_eq!(stream.tasks[0].busy_line(t0).as_deref(), Some("Gefunden"));
    // A plan's step comes first once there is one.
    stream.apply(
        &Event::Plan {
            conversation: id(1),
            steps: vec![PlanStep {
                title: "Tests schreiben".into(),
                done: false,
            }],
        },
        None,
        t0,
    );
    stream.apply(&search, None, t0);
    assert_eq!(
        stream.tasks[0].busy_line(t0).as_deref(),
        Some("Tests schreiben")
    );
    crate::desktop::i18n::apply(Some("en"));
}

#[test]
fn a_long_running_tool_says_how_long_it_runs() {
    crate::desktop::i18n::apply(Some("de"));
    let t0 = Instant::now();
    let mut stream = Stream::default();
    stream.apply(&started(1, Origin::Background, "Build it"), None, t0);
    stream.apply(
        &tool(1, "Bash", serde_json::json!({"command": "cargo build"})),
        None,
        t0,
    );
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    assert_eq!(
        stream.tasks[0].busy_line(at(9)).as_deref(),
        Some("Führt Befehl aus")
    );
    assert_eq!(
        stream.next_tick(at(3)),
        Some(at(10)),
        "first at ten seconds"
    );
    assert_eq!(
        stream.tasks[0].busy_line(at(80)).as_deref(),
        Some("Führt Befehl aus · 1:20")
    );
    assert_eq!(stream.next_tick(at(80)), Some(at(81)), "then each second");
    assert_eq!(elapsed(Duration::from_secs(3723)), "1:02:03");
    // Its output's tail goes to the terminal, and the clock stops.
    stream.apply(
        &Event::ToolDone {
            conversation: id(1),
            lines: console::output_lines(ToolKind::Command, "Compiling\nFinished"),
        },
        None,
        at(90),
    );
    assert_eq!(stream.next_tick(at(90)), None);
    assert_eq!(stream.tasks[0].console.tail(1)[0].text, "Finished");
    crate::desktop::i18n::apply(Some("en"));
}

#[test]
fn tools_are_read_by_their_kind_for_every_backend() {
    for (name, kind) in [
        ("Bash", ToolKind::Command),
        ("exec_command", ToolKind::Command),
        ("command_execution", ToolKind::Command),
        ("Grep", ToolKind::Search),
        ("Glob", ToolKind::Search),
        ("mcp__fs__list_directory", ToolKind::Search),
        ("Read", ToolKind::Read),
        ("mcp__fs__read_file", ToolKind::Read),
        ("Edit", ToolKind::Edit),
        ("Write", ToolKind::Edit),
        ("file_change", ToolKind::Edit),
        ("apply_patch", ToolKind::Edit),
        ("WebSearch", ToolKind::WebSearch),
        ("WebFetch", ToolKind::Fetch),
        ("mcp__agent-browser__browser_snapshot", ToolKind::Browser),
        ("mcp__desktop__desktop_click", ToolKind::Desktop),
        ("Task", ToolKind::Helper),
        ("TodoWrite", ToolKind::Plan),
        ("mcp__hex__hex_remember", ToolKind::Other),
    ] {
        assert_eq!(ToolKind::of(name), kind, "{name}");
    }
}
