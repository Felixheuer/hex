use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::assistant::computer::ContextHandle;
use crate::assistant::{Context, Origin};
use crate::control::PrintedTranscript;

fn read(text: &str) -> ContextHandle {
    let context = Context {
        frontmost_app: Some(text.into()),
        ..Context::default()
    };
    ContextHandle::start(move |reading| reading.quick(context))
}

fn nothing() -> ContextHandle {
    ContextHandle::start(|_| {})
}

/// A hold asks Here: it continues the focused conversation (the running
/// Here task, which it steers) with the desktop read at its press. A tap
/// then a hold starts its own Background task, without one.
#[test]
fn a_hold_asks_with_its_lane_settled_at_the_press() {
    let quick = Question::press(false, || read("Mail"));
    assert_eq!(quick.lane(), Lane::Here);
    let quick = quick
        .request("  What's this email about? ", || Some("c1".into()))
        .expect("a quick question");
    assert_eq!(quick.origin, Origin::ActionShortcut);
    assert_eq!(quick.conversation.as_deref(), Some("c1"));
    assert_eq!(quick.text, "What's this email about?");
    assert_eq!(quick.context.frontmost_app.as_deref(), Some("Mail"));

    let task = Question::press(true, || unreachable!("a Background task reads nothing"));
    assert_eq!(task.lane(), Lane::Background);
    let task = task
        .request("File these receipts", || {
            unreachable!("a Background task continues none")
        })
        .expect("a task");
    assert_eq!(task.origin, Origin::Background);
    assert_eq!(task.conversation, None);
    assert_eq!(task.context.frontmost_app, None);
}

/// A press reads nothing until the hold is real, and a press discarded
/// before then (a chord) never reads at all.
#[test]
fn the_desktop_is_read_only_once_the_hold_is_real() {
    let (started, reads) = mpsc::channel();
    let counted = move |sender: mpsc::Sender<()>| {
        move || {
            let _ = sender.send(());
            read("Mail")
        }
    };
    drop(Question::press(false, counted(started.clone())));
    assert!(reads.try_recv().is_err(), "a chord read the desktop");

    let mut question = Question::press(false, counted(started));
    assert!(reads.try_recv().is_err(), "the press read the desktop");
    // Only the first says the hold just became real (prewarm once).
    assert!(question.intend());
    assert!(!question.intend());
    assert_eq!(reads.try_iter().count(), 1);
    let asked = question
        .request("What is this?", || None)
        .expect("a question");
    assert_eq!(asked.context.frontmost_app.as_deref(), Some("Mail"));
}

/// Silence asks nothing.
#[test]
fn no_words_ask_nothing() {
    assert!(
        Question::press(false, nothing)
            .request("  \n", || None)
            .is_none()
    );
}

/// The transcript reaches `submit` from the question's own thread; a
/// dropped reply (a short tap, a discard, a cancel) and an empty
/// transcript submit nothing.
#[test]
fn the_transcript_submits_and_a_dropped_reply_asks_nothing() {
    let (submitted, asked) = mpsc::channel();
    let submit = move |sender: mpsc::Sender<Request>| {
        move |request| {
            let _ = sender.send(request);
        }
    };
    let reply = Question::press(false, nothing).ask(|| None, submit(submitted.clone()));
    reply.send(PrintedTranscript::Text("Summarize this page".into()));
    let request = asked
        .recv_timeout(Duration::from_secs(5))
        .expect("the question is asked");
    assert_eq!(request.origin, Origin::ActionShortcut);
    assert_eq!(request.text, "Summarize this page");

    drop(Question::press(true, nothing).ask(|| None, submit(submitted.clone())));
    Question::press(false, nothing)
        .ask(|| None, submit(submitted.clone()))
        .send(PrintedTranscript::NoSpeech);
    Question::press(true, nothing)
        .ask(|| None, submit(submitted))
        .send(PrintedTranscript::Text("   ".into()));
    assert!(asked.recv_timeout(Duration::from_millis(200)).is_err());
}

/// "In the background …" sends a Here question to the Background lane
/// without those words, in German or English, whatever the case or the
/// punctuation around them.
#[test]
fn spoken_openings_route_to_the_background() {
    for (said, rest) in [
        (
            "Im Hintergrund räum den Downloads-Ordner auf",
            "räum den Downloads-Ordner auf",
        ),
        ("im hintergrund, räum auf.", "räum auf."),
        ("IM HINTERGRUND: Mails sortieren", "Mails sortieren"),
        ("  „Im Hintergrund“ – buch den Flug", "buch den Flug"),
        (
            "In the background, file these receipts",
            "file these receipts",
        ),
        ("in the Background: check my calendar", "check my calendar"),
        (
            "In the background... summarise the PDF",
            "summarise the PDF",
        ),
        ("Nebenbei: schreib Anna zurück", "schreib Anna zurück"),
        ("nebenbei bestell Kaffee", "bestell Kaffee"),
        ("Parallel, update the spreadsheet", "update the spreadsheet"),
        (
            "parallel lad die Rechnungen herunter",
            "lad die Rechnungen herunter",
        ),
        ("Hintergrund: Downloads aufräumen", "Downloads aufräumen"),
        ("hintergrund : Downloads aufräumen", "Downloads aufräumen"),
        ("Im Hintergrund", ""),
        ("In the background.", ""),
    ] {
        assert_eq!(spoken_background(said), Some(rest), "{said:?}");
    }
}

/// Only the whole words open it: a word that merely starts like one, the
/// words later in the sentence, or "Hintergrund" without its colon ask
/// about the window in front.
#[test]
fn words_that_merely_contain_an_opening_stay_here() {
    for said in [
        "Parallelogramm zeichnen",
        "Parallels Desktop öffnen",
        "Hintergrundbild ändern",
        "Hintergrund ändern",
        "Hintergrund-Prozesse anzeigen",
        "Nebenbeigesagt, was steht hier?",
        "Im Hintergrundbild ist was?",
        "Imhintergrund aufräumen",
        "In the backgrounds folder, what's new?",
        "In the back, what's that?",
        "In thebackground",
        "What runs in the background?",
        "Mach das parallel",
        "",
        "   ",
    ] {
        assert_eq!(spoken_background(said), None, "{said:?}");
    }
}

/// A Here question that opens with "im Hintergrund" becomes a new
/// Background task: no conversation, no screenshot, and the read its hold
/// started is dropped. Nothing after the opening asks nothing.
#[test]
fn a_spoken_opening_asks_a_background_task() {
    let mut question = Question::press(false, || read("Mail"));
    question.intend();
    let task = question
        .request("Im Hintergrund, räum die Downloads auf", || {
            unreachable!("a Background task continues none")
        })
        .expect("a task");
    assert_eq!(task.origin, Origin::Background);
    assert_eq!(task.conversation, None);
    assert_eq!(task.text, "räum die Downloads auf");
    assert!(task.context.screenshot.is_none());
    assert_eq!(task.context.frontmost_app, None);

    assert!(
        Question::press(false, nothing)
            .request("In the background.", || None)
            .is_none()
    );
    // After a tap the words are the task's, opening and all.
    let task = Question::press(true, nothing)
        .request("Parallelogramm zeichnen", || None)
        .expect("a task");
    assert_eq!(task.text, "Parallelogramm zeichnen");
}

fn task(conversation: &str, origin: Origin) -> TaskStatus {
    TaskStatus {
        conversation: conversation.into(),
        origin,
        title: conversation.into(),
        state: crate::assistant::TaskState::Running,
        step: None,
        windows: Vec::new(),
        colour: 0,
    }
}

/// Escape stops the Here tasks only: never a Background or Chats task.
#[test]
fn escape_stops_only_here_tasks() {
    let tasks = [
        task("background", Origin::Background),
        task("here", Origin::ActionShortcut),
        task("chat", Origin::Chat),
    ];
    assert_eq!(here_tasks(&tasks), ["here"]);
    assert!(here_tasks(&tasks[..1]).is_empty());
    assert!(here_tasks(&[]).is_empty());
}

/// The Stop all key presses once per press: its repeats and its release
/// stop nothing, another key or the key without its modifiers is no press,
/// and the next press after its release counts again.
#[test]
fn the_stop_all_key_presses_once_per_press() {
    const ESCAPE: u16 = 27;
    let mut key = StopAllKey::default();
    assert!(key.press(ESCAPE, true, true));
    assert!(key.is_down());
    assert!(!key.press(ESCAPE, true, true), "a repeat");
    assert!(!key.press(ESCAPE, false, true));
    assert!(!key.is_down());
    assert!(key.press(ESCAPE, true, true));
    assert!(!key.press(ESCAPE, false, false));
    assert!(!key.press(ESCAPE, true, false), "Escape alone");
    assert!(!key.press(12, true, false));
    assert!(!key.is_down());
}

/// A double tap calls what the overlays set; until then it does nothing.
#[test]
fn a_double_tap_calls_the_task_list_toggle() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static TOGGLED: AtomicUsize = AtomicUsize::new(0);
    toggle_tasks();
    set_toggle_tasks(|| {
        TOGGLED.fetch_add(1, Ordering::SeqCst);
    });
    toggle_tasks();
    toggle_tasks();
    assert_eq!(TOGGLED.load(Ordering::SeqCst), 2);
}

/// Before the setup is finished only a hold opens it: a tap or a chord
/// released before the hold threshold never does, and the hook runs once
/// it is set.
#[test]
fn only_a_hold_opens_the_setup() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let at = |ms: u64| CaptureInstant::ZERO + Duration::from_millis(ms);
    assert!(held(at(1_000), at(1_300)));
    assert!(held(at(1_000), at(4_000)));
    assert!(!held(at(1_000), at(1_080)));
    assert!(!held(at(1_000), at(1_299)));
    assert!(!held(at(1_000), at(900)));

    static OPENED: AtomicUsize = AtomicUsize::new(0);
    open_setup();
    set_open_setup(|| {
        OPENED.fetch_add(1, Ordering::SeqCst);
    });
    open_setup();
    assert_eq!(OPENED.load(Ordering::SeqCst), 1);
}

/// Before setup a press records nothing and its release asks nothing; a
/// double tap then does nothing either.
#[test]
fn a_press_before_setup_asks_nothing() {
    let mut asking = Asking {
        setup_press: Some(CaptureInstant::ZERO),
        ..Asking::default()
    };
    assert!(asking.setting_up());
    assert!(
        asking
            .finish(CaptureInstant::ZERO + Duration::from_millis(50))
            .is_none()
    );
    assert!(!asking.setting_up());
    asking.setup_press = Some(CaptureInstant::ZERO);
    asking.double_tap();
    assert!(!asking.setting_up());
}

// ---------------------------------------------------------------------------
// The gestures Windows and Linux tell apart (`Gesture`), as macOS's own
// machine does (`recognition::hotkeys`).

/// Feeds presses (`true`) and releases at `ms` and returns what each did.
fn gestures(steps: &[(bool, u64)]) -> Vec<Option<GestureAction>> {
    let base = Instant::now();
    let mut gesture = Gesture::default();
    steps
        .iter()
        .map(|&(down, ms)| {
            let at = base + Duration::from_millis(ms);
            if down {
                gesture.press(at)
            } else {
                gesture.release(at)
            }
        })
        .collect()
}

const fn start(after_tap: bool) -> Option<GestureAction> {
    Some(GestureAction::Start { after_tap })
}

const FINISH: Option<GestureAction> = Some(GestureAction::Finish);
const DOUBLE_TAP: Option<GestureAction> = Some(GestureAction::DoubleTap);

/// A hold starts and finishes; a second hold right after asks Here again.
#[test]
fn a_hold_asks_here() {
    assert_eq!(
        gestures(&[(true, 0), (false, 1_500), (true, 1_600), (false, 3_000)]),
        [start(false), FINISH, start(false), FINISH]
    );
}

/// A tap, then within the double-tap window a press held past the hold
/// threshold: that press follows a tap, and its release finishes it. The
/// next hold is Here again, and a second press just past the window is a
/// plain hold.
#[test]
fn a_tap_then_a_hold_asks_in_the_background() {
    assert_eq!(
        gestures(&[
            (true, 0),
            (false, 70),
            (true, 200),
            (false, 2_000),
            (true, 2_100),
            (false, 3_500),
            (true, 6_000),
            (false, 6_070),
            (true, 6_400),
            (false, 8_000),
        ]),
        [
            start(false),
            FINISH,
            start(true),
            FINISH,
            start(false),
            FINISH,
            start(false),
            FINISH,
            start(false),
            FINISH,
        ]
    );
}

/// Two taps are the double tap; a third tap starts over rather than pairing
/// with it. A second tap released after the first's window closed is still
/// a tap, never a hold.
#[test]
fn a_double_tap_is_a_gesture() {
    assert_eq!(
        gestures(&[
            (true, 0),
            (false, 70),
            (true, 150),
            (false, 220),
            (true, 300),
            (false, 360),
            (true, 2_000),
            (false, 2_060),
            (true, 2_300),
            (false, 2_420),
        ]),
        [
            start(false),
            FINISH,
            start(true),
            DOUBLE_TAP,
            start(false),
            FINISH,
            start(false),
            FINISH,
            start(true),
            DOUBLE_TAP,
        ]
    );
}

/// A hold, then a quick tap: the hold asked, and the tap pairs with
/// nothing. Repeats while down and a release without a press do nothing.
#[test]
fn a_hold_never_pairs_with_the_next_press() {
    assert_eq!(
        gestures(&[
            (false, 0),
            (true, 6_000),
            (true, 6_100),
            (false, 7_000),
            (false, 7_010),
            (true, 7_080),
            (false, 7_150),
        ]),
        [None, start(false), None, FINISH, None, start(false), FINISH]
    );
}

/// A cancelled press leaves nothing armed: the next press is a fresh one.
#[test]
fn a_cancelled_press_arms_nothing() {
    let base = Instant::now();
    let at = |ms: u64| base + Duration::from_millis(ms);
    let mut gesture = Gesture::default();
    assert_eq!(gesture.press(at(0)), start(false));
    assert_eq!(gesture.release(at(60)), FINISH);
    assert_eq!(gesture.press(at(120)), start(true));
    assert!(gesture.cancel());
    assert!(!gesture.cancel());
    assert_eq!(gesture.release(at(150)), None);
    assert_eq!(gesture.press(at(200)), start(false));
}

/// A press made before setup records nothing however it ends; once a
/// question is under way, a cancel ends its capture and asks nothing.
/// (Windows and Linux: macOS's listener carries its events out itself.)
#[cfg(not(target_os = "macos"))]
#[test]
fn the_listeners_events_follow_the_setup() {
    let at = |ms: u64| CaptureInstant::ZERO + Duration::from_millis(ms);
    for event in [Event::Finish, Event::Cancel, Event::DoubleTap] {
        let mut asking = Asking {
            setup_press: Some(at(0)),
            ..Asking::default()
        };
        assert!(
            matches!(asking.take(event, at(50)), Step::Done),
            "{event:?}"
        );
        assert!(!asking.setting_up());
    }
    let mut asking = Asking {
        question: Some(Question::press(false, nothing)),
        ..Asking::default()
    };
    assert!(matches!(asking.take(Event::Cancel, at(80)), Step::Cancel));
    assert!(asking.question.is_none());
}
