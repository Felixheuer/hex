//! The runner with a fake backend and a fake clock: lanes and queueing,
//! steering, the frame and its desktop session, take over, timeouts, usage
//! limits, cleanup on every end, follow-ups, and speech.

use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::assistant::types::{Context, TaskWindow};

/// What the fake backend does with a question.
#[derive(Clone)]
enum Script {
    /// Waits until released or cancelled, then answers.
    Answer,
    /// Hits a limit on its first run (lifting at this clock time, or
    /// unknown), then answers.
    Limit(Option<u64>),
    /// Panics.
    Panic,
    /// Streams these pieces of answer, then answers with all of them.
    Stream(Vec<&'static str>),
}

/// A fake world: tasks wait until their question is released or they are
/// cancelled; everything the runner did is recorded.
struct World {
    released: Mutex<HashSet<String>>,
    scripts: Mutex<Vec<(String, Script)>>,
    events: Mutex<Vec<Event>>,
    log: Mutex<Vec<String>>,
    jobs: Mutex<Vec<Job>>,
    spoken: Mutex<Vec<String>>,
    lab_on: Mutex<bool>,
    settings: Mutex<AssistantSettings>,
    clock: AtomicU64,
    hands_off: Mutex<Option<String>>,
    recent: Mutex<Vec<ConversationSummary>>,
    interrupted: Mutex<Vec<(String, Origin, String)>>,
    requests: Mutex<Vec<(String, remote::Verb)>>,
}

const START: u64 = 1_000_000;

impl World {
    fn new(max_parallel: u32) -> Arc<Self> {
        let mut settings = AssistantSettings {
            max_parallel,
            setup_done: true,
            ..AssistantSettings::default()
        };
        settings.task_timeout_minutes = 0;
        Arc::new(Self {
            released: Mutex::default(),
            scripts: Mutex::default(),
            events: Mutex::default(),
            log: Mutex::default(),
            jobs: Mutex::default(),
            spoken: Mutex::default(),
            lab_on: Mutex::new(true),
            settings: Mutex::new(settings),
            clock: AtomicU64::new(START),
            hands_off: Mutex::default(),
            recent: Mutex::default(),
            interrupted: Mutex::default(),
            requests: Mutex::default(),
        })
    }

    fn log(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }

    fn logged(&self, line: &str) -> bool {
        self.log.lock().unwrap().iter().any(|logged| logged == line)
    }

    fn script(&self, question: &str, script: Script) {
        self.scripts
            .lock()
            .unwrap()
            .push((question.to_owned(), script));
    }

    fn execute(&self, job: &Job, control: &Control, emit: &mut dyn FnMut(Event)) -> Outcome {
        self.jobs.lock().unwrap().push(job.clone());
        let script = self
            .scripts
            .lock()
            .unwrap()
            .iter()
            .find(|(question, _)| *question == job.request.text)
            .map(|(_, script)| script.clone())
            .unwrap_or(Script::Answer);
        match script {
            Script::Panic => panic!("the fake backend fell over"),
            Script::Limit(until_ms) if job.kind != JobKind::AfterLimit => {
                return Outcome::Limited {
                    reason: "You've hit your session limit".into(),
                    until_ms,
                };
            }
            Script::Stream(pieces) => {
                for piece in &pieces {
                    emit(Event::AnswerDelta {
                        conversation: job.conversation.clone(),
                        text: (*piece).into(),
                    });
                }
                return Outcome::Answer(pieces.concat());
            }
            _ => {}
        }
        emit(Event::Step {
            conversation: job.conversation.clone(),
            description: format!("Working on {}", job.request.text),
        });
        emit(Event::Window {
            conversation: job.conversation.clone(),
            window: TaskWindow {
                app: "Mail".into(),
                window_id: Some("w-1".into()),
            },
        });
        loop {
            if control.cancelled() {
                return Outcome::Cancelled;
            }
            if self.released.lock().unwrap().contains(&job.request.text) {
                return Outcome::Answer(format!("answer to {}", job.request.text));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn runner(self: &Arc<Self>) -> Arc<Runner> {
        let world = |world: &Arc<Self>| Arc::clone(world);
        let (execute, settings, start, end, style, speak) = (
            world(self),
            world(self),
            world(self),
            world(self),
            world(self),
            world(self),
        );
        let (clock, pause, release, hands, forward, settle) = (
            world(self),
            world(self),
            world(self),
            world(self),
            world(self),
            world(self),
        );
        let (recent, interrupted, prewarm, requests) =
            (world(self), world(self), world(self), world(self));
        let runner = Runner::new(Hooks {
            execute: Box::new(move |job, control, emit| execute.execute(job, control, emit)),
            settings: Box::new(move || {
                (
                    *settings.lab_on.lock().unwrap(),
                    settings.settings.lock().unwrap().clone(),
                )
            }),
            start_session: Box::new(move || {
                start.log("start session".into());
                Some("S1".into())
            }),
            end_session: Box::new(move |id| end.log(format!("end session {id}"))),
            style_agent: Box::new(move |session, agent, colour| {
                style.log(format!("style {session} {agent} {colour}"));
            }),
            speak: Box::new(move |text, _| speak.spoken.lock().unwrap().push(text.to_owned())),
            speaking: Box::new(|| false),
            pick_voice: Box::new(|_| None),
            stop_speaking: Box::new(|| {}),
            now_ms: Box::new(move || clock.clock.load(Ordering::SeqCst)),
            sleep: Box::new(|_| std::thread::sleep(Duration::from_millis(1))),
            pause: Box::new(move |task, paused| pause.log(format!("pause {task} {paused}"))),
            release: Box::new(move |task| release.log(format!("release {task}"))),
            hands_off: Box::new(move || hands.hands_off.lock().unwrap().clone()),
            bring_forward: Box::new(move |task, windows| {
                forward.log(format!("forward {task} {}", windows.len()));
            }),
            settle: Box::new(move |task, outcome, message| {
                settle.log(format!("settle {task} {outcome:?} {message}"));
            }),
            recent: Box::new(move || recent.recent.lock().unwrap().clone()),
            interrupted: Box::new(move |id| {
                interrupted
                    .interrupted
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(found, _, _)| found == id)
                    .map(|(_, origin, text)| (*origin, text.clone()))
            }),
            prewarm: Box::new(move |job| {
                prewarm.log(format!(
                    "prewarm {} {} {:?}",
                    job.conversation, job.agent_id, job.request.origin
                ));
            }),
            requests: Box::new(move || std::mem::take(&mut *requests.requests.lock().unwrap())),
        });
        let events = Arc::clone(self);
        runner.subscribe(Box::new(move |event| {
            events.events.lock().unwrap().push(event.clone())
        }));
        runner
    }

    fn release(&self, question: &str) {
        self.released.lock().unwrap().insert(question.into());
    }

    fn advance(&self, ms: u64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn wait_for(&self, what: &str, done: impl Fn(&[Event]) -> bool) {
        let started = Instant::now();
        while !done(&self.events.lock().unwrap()) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}: {:#?}\n{:#?}",
                self.events.lock().unwrap(),
                self.log.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_until(&self, what: &str, done: impl Fn() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}: {:#?}",
                self.log.lock().unwrap()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn finals(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.is_final())
            .cloned()
            .collect()
    }

    fn modes(&self) -> Vec<bool> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::ModeChanged { on } => Some(*on),
                _ => None,
            })
            .collect()
    }

    fn count(&self, matches: impl Fn(&Event) -> bool) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches(event))
            .count()
    }
}

fn request(text: &str, origin: Origin) -> Request {
    Request {
        conversation: None,
        text: text.into(),
        origin,
        context: Context::default(),
    }
}

fn started(id: &str) -> impl Fn(&[Event]) -> bool + '_ {
    move |events| {
        events
            .iter()
            .any(|event| matches!(event, Event::Started { conversation, .. } if conversation == id))
    }
}

fn ended(id: &str) -> impl Fn(&[Event]) -> bool + '_ {
    move |events| {
        events
            .iter()
            .any(|event| event.is_final() && event.conversation().map(String::as_str) == Some(id))
    }
}

#[test]
fn background_tasks_beyond_max_parallel_wait_their_turn() {
    let world = World::new(2);
    let runner = world.runner();
    let a = runner.submit(request("a", Origin::Chat));
    let b = runner.submit(request("b", Origin::Background));
    let c = runner.submit(request("c", Origin::Chat));
    // The Here lane has its own slot.
    let here = runner.submit(request("here", Origin::ActionShortcut));
    world.wait_for("a, b, and the Here task to start", |events| {
        started(&a)(events) && started(&b)(events) && started(&here)(events)
    });

    let states: Vec<_> = runner
        .tasks()
        .into_iter()
        .map(|task| (task.conversation, task.state))
        .collect();
    assert_eq!(
        states,
        [
            (a.clone(), TaskState::Running),
            (b.clone(), TaskState::Running),
            (c.clone(), TaskState::Queued),
            (here.clone(), TaskState::Running),
        ]
    );
    let colours: Vec<u8> = runner.tasks().iter().map(|task| task.colour).collect();
    assert_eq!(colours, [0, 1, 2, 3]);
    world.wait_for("a's step", |_| {
        runner.tasks()[0].step.as_deref() == Some("Working on a")
            && runner.tasks()[0].windows.len() == 1
    });
    assert!(!started(&c)(&world.events.lock().unwrap()));

    world.release("b");
    world.wait_for("c to start once b is done", started(&c));
    world.release("a");
    world.release("c");
    world.release("here");
    world.wait_for("every answer", |events| {
        [&a, &b, &c, &here].iter().all(|id| ended(id)(events))
    });
    assert!(runner.tasks().is_empty());
    // Every end released the task's windows and browser.
    for id in [&a, &b, &c, &here] {
        assert!(world.logged(&format!("release {id}")), "{id}");
        assert!(world.logged(&format!("settle {id} Answered ")), "{id}");
    }
}

#[test]
fn a_new_here_request_stops_the_last_one_but_not_background_tasks() {
    let world = World::new(8);
    let runner = world.runner();
    let background = runner.submit(request("background", Origin::Background));
    let first = runner.submit(request("first", Origin::ActionShortcut));
    world.wait_for("the first Here task", started(&first));
    let second = runner.submit(request("second", Origin::ActionShortcut));
    world.wait_for("the first to be cancelled", ended(&first));
    world.wait_for("the second to start", started(&second));
    assert_eq!(
        world.finals(),
        [Event::Cancelled {
            conversation: first.clone()
        }]
    );
    assert!(world.logged(&format!("settle {first} Cancelled Stopped.")));
    assert!(world.logged(&format!("release {first}")));

    world.release("second");
    world.wait_for("the second answer", ended(&second));
    assert_eq!(*world.spoken.lock().unwrap(), ["answer to second"]);
    assert_eq!(runner.tasks().len(), 1);
    assert_eq!(runner.tasks()[0].conversation, background);
    runner.cancel(&background);
    world.wait_for("the background task to stop", ended(&background));
}

#[test]
fn speaking_into_a_running_task_steers_it_in_the_same_conversation() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("write to Anna", Origin::Background));
    world.wait_for("the task", started(&id));
    world.wait_for("its step", |_| runner.tasks()[0].step.is_some());

    let mut steer = request("use Friday instead", Origin::ActionShortcut);
    steer.conversation = Some(id.clone());
    assert_eq!(runner.submit(steer), id);
    world.wait_until("the steered run", || world.jobs.lock().unwrap().len() == 2);
    let job = world.jobs.lock().unwrap()[1].clone();
    assert_eq!(job.conversation, id);
    assert_eq!(job.request.text, "use Friday instead");
    // It stays in its lane.
    assert_eq!(job.request.origin, Origin::Background);
    assert_eq!(
        job.kind,
        JobKind::Steer {
            doing: Some("Working on write to Anna".into()),
            steps: vec!["Working on write to Anna".into()],
        }
    );
    // The stopped turn kept its windows and browser, and said nothing final.
    assert!(!world.logged(&format!("release {id}")));
    assert!(world.finals().is_empty());

    world.release("use Friday instead");
    world.wait_for("the answer", ended(&id));
    assert_eq!(
        world.finals(),
        [Event::Answer {
            conversation: id.clone(),
            origin: Origin::Background,
            text: "answer to use Friday instead".into()
        }]
    );
    assert_eq!(world.count(|event| matches!(event, Event::Started { .. })), 2);
}

#[test]
fn a_request_naming_a_running_task_is_routed_to_it() {
    let world = World::new(8);
    let runner = world.runner();
    let mail = runner.submit(request("Antworte Anna auf ihre Mail", Origin::Background));
    let invoice = runner.submit(request("Lade die Hetzner Rechnung", Origin::Background));
    world.wait_for("both", |events| started(&mail)(events) && started(&invoice)(events));
    let routed = runner.submit(request("bei der Anna-Aufgabe nimm Freitag", Origin::ActionShortcut));
    assert_eq!(routed, mail);
    world.wait_until("the steered run", || world.jobs.lock().unwrap().len() == 3);
    // Anything else starts a new task that knows what else is going on.
    let other = runner.submit(request("mach das auch für Bob", Origin::Background));
    assert_ne!(other, mail);
    world.wait_until("the new task", || world.jobs.lock().unwrap().len() == 4);
    let job = world.jobs.lock().unwrap()[3].clone();
    let digest = job.digest.unwrap();
    assert!(digest.contains("“Lade die Hetzner Rechnung” (background)"), "{digest}");
    runner.cancel_all();
    world.wait_for("all to stop", |events| {
        [&mail, &invoice, &other].iter().all(|id| ended(id)(events))
    });
}

#[test]
fn the_frame_shows_while_background_work_runs_with_one_desktop_session() {
    let world = World::new(1);
    let runner = world.runner();
    let here = runner.submit(request("here", Origin::ActionShortcut));
    world.wait_for("the Here task", started(&here));
    assert!(world.modes().is_empty());
    assert!(!runner.state().background_on);

    let first = runner.submit(request("first", Origin::Background));
    let queued = runner.submit(request("queued", Origin::Chat));
    assert_eq!(world.modes(), [true]);
    world.wait_for("the first background task", started(&first));
    world.wait_until("its job", || world.jobs.lock().unwrap().len() == 2);
    let job = world.jobs.lock().unwrap()[1].clone();
    assert_eq!(job.desktop_session.as_deref(), Some("S1"));
    assert!(world.logged(&format!("style S1 {} 1", job.agent_id)));

    world.release("first");
    world.wait_for("the queued task", started(&queued));
    // Still on while the queued one runs.
    assert_eq!(world.modes(), [true]);
    world.release("queued");
    world.wait_for("the queued answer", ended(&queued));
    world.wait_until("the frame to go off", || world.modes() == [true, false]);
    assert!(!runner.state().background_on);
    // The Here task still runs, so the session stays until it ends.
    assert!(!world.logged("end session S1"));
    world.release("here");
    world.wait_for("the Here answer", ended(&here));
    world.wait_until("the session to end", || world.logged("end session S1"));

    // The old switch: off stops every task.
    let more = runner.submit(request("more", Origin::Background));
    world.wait_for("another", started(&more));
    runner.cancel_all();
    world.wait_for("it to stop", ended(&more));
    world.wait_until("the frame off again", || world.modes() == [true, false, true, false]);
}

#[test]
fn take_over_pauses_and_hand_back_resumes() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("task", Origin::Background));
    world.wait_for("the task", started(&id));
    world.wait_for("its window", |_| runner.tasks()[0].windows.len() == 1);
    runner.take_over(&id);
    assert_eq!(runner.tasks()[0].state, TaskState::Paused);
    assert!(world.logged(&format!("pause {id} true")));
    assert!(world.logged(&format!("forward {id} 1")));
    runner.take_over(&id);
    runner.hand_back(&id);
    assert_eq!(runner.tasks()[0].state, TaskState::Running);
    assert!(world.logged(&format!("pause {id} false")));
    let changes: Vec<Event> = world
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, Event::Paused { .. } | Event::Resumed { .. }))
        .cloned()
        .collect();
    assert_eq!(
        changes,
        [
            Event::Paused {
                conversation: id.clone()
            },
            Event::Resumed {
                conversation: id.clone()
            }
        ]
    );
    // Ending while taken over hands it back to the guard too.
    runner.take_over(&id);
    runner.cancel(&id);
    world.wait_for("the end", ended(&id));
    assert_eq!(
        world
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|line| **line == format!("pause {id} false"))
            .count(),
        2
    );
    assert!(world.logged(&format!("release {id}")));
}

#[test]
fn hands_off_follows_the_guard_while_a_here_task_runs() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("click it", Origin::ActionShortcut));
    world.wait_for("the task", started(&id));
    *world.hands_off.lock().unwrap() = Some(id.clone());
    world.wait_for("hands off", |events| {
        events.contains(&Event::HandsOff {
            conversation: id.clone(),
            on: true,
        })
    });
    world.release("click it");
    world.wait_for("the answer", ended(&id));
    world.wait_for("hands off to end", |events| {
        events.contains(&Event::HandsOff {
            conversation: id.clone(),
            on: false,
        })
    });
}

#[test]
fn a_task_running_too_long_fails_with_a_clear_message() {
    let world = World::new(8);
    world.settings.lock().unwrap().task_timeout_minutes = 30;
    let runner = world.runner();
    let id = runner.submit(request("slow", Origin::Background));
    world.wait_for("the task", started(&id));
    world.wait_until("its deadline", || {
        runner.state().tasks[0].deadline_ms == Some(START + 30 * 60_000)
    });
    // Time taken over doesn't count.
    runner.take_over(&id);
    world.advance(40 * 60_000);
    std::thread::sleep(Duration::from_millis(20));
    runner.hand_back(&id);
    assert!(!ended(&id)(&world.events.lock().unwrap()));
    world.advance(31 * 60_000);
    world.wait_for("the timeout", ended(&id));
    let message = "Stopped after 30 minutes: a task may run at most 30 minutes (Settings › Assistant).";
    assert_eq!(
        world.finals(),
        [Event::Failed {
            conversation: id.clone(),
            message: message.into()
        }]
    );
    assert!(world.logged(&format!("settle {id} Failed {message}")));
    assert!(world.logged(&format!("release {id}")));
}

#[test]
fn a_usage_limit_waits_for_its_reset_then_retries() {
    let world = World::new(8);
    world.script("busy", Script::Limit(Some(START + 3_600_000)));
    let runner = world.runner();
    let id = runner.submit(request("busy", Origin::Background));
    world.wait_for("the wait", |events| {
        events.iter().any(|event| matches!(event, Event::Waiting { .. }))
    });
    assert_eq!(
        runner.tasks()[0].state,
        TaskState::Waiting {
            until_ms: START + 3_600_000
        }
    );
    assert!(world.events.lock().unwrap().contains(&Event::Waiting {
        conversation: id.clone(),
        until_ms: START + 3_600_000,
        reason: "You've hit your session limit".into()
    }));
    // Still waiting a minute before the reset; the frame stays on.
    world.advance(3_540_000);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(world.jobs.lock().unwrap().len(), 1);
    assert!(runner.state().background_on);
    world.advance(60_000);
    world.wait_until("the retry", || world.jobs.lock().unwrap().len() == 2);
    assert_eq!(world.jobs.lock().unwrap()[1].kind, JobKind::AfterLimit);
    assert_eq!(runner.tasks()[0].state, TaskState::Running);
    world.release("busy");
    world.wait_for("the answer", ended(&id));
    assert!(matches!(world.finals()[0], Event::Answer { .. }));
}

#[test]
fn a_limit_without_a_reset_backs_off_and_stopping_while_waiting_cleans_up() {
    let world = World::new(8);
    world.script("busy", Script::Limit(None));
    let runner = world.runner();
    let id = runner.submit(request("busy", Origin::Chat));
    world.wait_for("the wait", |events| {
        events.iter().any(|event| {
            matches!(event, Event::Waiting { until_ms, .. } if *until_ms == START + 60_000)
        })
    });
    runner.cancel(&id);
    world.wait_for("the end", ended(&id));
    assert_eq!(world.finals(), [Event::Cancelled { conversation: id.clone() }]);
    assert!(world.logged(&format!("settle {id} Cancelled Stopped.")));
    assert!(world.logged(&format!("release {id}")));
    assert_eq!(backoff(1), 60_000);
    assert_eq!(backoff(3), 240_000);
    assert_eq!(backoff(30), 30 * 60_000);
}

#[test]
fn a_task_that_panics_fails_and_still_cleans_up() {
    let world = World::new(8);
    world.script("boom", Script::Panic);
    let runner = world.runner();
    let id = runner.submit(request("boom", Origin::Background));
    world.wait_for("the failure", ended(&id));
    assert_eq!(
        world.finals(),
        [Event::Failed {
            conversation: id.clone(),
            message: "The task stopped unexpectedly.".into()
        }]
    );
    assert!(world.logged(&format!("release {id}")));
    world.wait_until("the frame off", || world.modes() == [true, false]);
}

#[test]
fn cancelling_a_queued_task_ends_it_without_starting() {
    let world = World::new(1);
    let runner = world.runner();
    let a = runner.submit(request("a", Origin::Chat));
    let b = runner.submit(request("b", Origin::Chat));
    world.wait_for("a to start", started(&a));
    runner.cancel(&b);
    assert_eq!(
        world.finals(),
        [Event::Cancelled {
            conversation: b.clone()
        }]
    );
    assert!(world.logged(&format!("release {b}")));
    world.release("a");
    world.wait_for("a's answer", ended(&a));
    assert!(!started(&b)(&world.events.lock().unwrap()));
}

#[test]
fn an_interrupted_conversation_resumes_with_a_note_to_look_first() {
    let world = World::new(8);
    world.interrupted.lock().unwrap().push((
        "c-old".into(),
        Origin::ActionShortcut,
        "Schreib Anna".into(),
    ));
    let runner = world.runner();
    assert_eq!(runner.resume_interrupted("c-other"), None);
    assert_eq!(runner.resume_interrupted("c-old").as_deref(), Some("c-old"));
    world.wait_until("the run", || world.jobs.lock().unwrap().len() == 1);
    let job = world.jobs.lock().unwrap()[0].clone();
    assert_eq!(job.kind, JobKind::Resume);
    assert_eq!(job.request.text, "Schreib Anna");
    // Its window may be gone: it goes on in the Background lane.
    assert_eq!(job.request.origin, Origin::Background);
    assert_eq!(job.digest, None);
    world.release("Schreib Anna");
    world.wait_for("the answer", ended("c-old"));
}

#[test]
fn a_here_answer_is_read_aloud_sentence_by_sentence() {
    let world = World::new(8);
    world.script(
        "time",
        Script::Stream(vec![
            "Es ist 18 Uhr in Tokio. ",
            "Dort ist es schon dunkel. Und",
            " kalt",
        ]),
    );
    let runner = world.runner();
    let id = runner.submit(request("time", Origin::ActionShortcut));
    world.wait_for("the answer", ended(&id));
    world.wait_until("all of it spoken", || world.spoken.lock().unwrap().len() == 3);
    assert_eq!(
        *world.spoken.lock().unwrap(),
        [
            "Es ist 18 Uhr in Tokio.",
            "Dort ist es schon dunkel.",
            "Und kalt"
        ]
    );
}

#[test]
fn focus_prefers_a_running_here_task_then_chats_then_a_recent_answer() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("q", Origin::ActionShortcut));
    world.wait_for("the task", started(&id));
    runner.state().focus = Some("chat".into());
    assert_eq!(runner.focus(120), Some(id.clone()));
    world.release("q");
    world.wait_for("the answer", ended(&id));
    assert_eq!(runner.focus(120).as_deref(), Some("chat"));
    runner.state().focus = None;
    assert_eq!(runner.focus(120), Some(id.clone()));
    world.advance(121_000);
    assert_eq!(runner.focus(120), None);
    assert_eq!(runner.focus(0), None);
}

#[test]
fn prewarm_reserves_the_next_requests_conversation_and_agent() {
    let world = World::new(8);
    world.settings.lock().unwrap().backend = AssistantBackend::ClaudeCode;
    let runner = world.runner();
    let window = Some(TaskWindow {
        app: "Mail".into(),
        window_id: Some("w-9".into()),
    });
    runner.prewarm(Lane::Here, window.clone());
    world.wait_until("the prewarm", || {
        world.log.lock().unwrap().iter().any(|line| line.starts_with("prewarm"))
    });
    let (id, agent) = {
        let state = runner.state();
        let warm = state.warm.as_ref().unwrap();
        (warm.id.clone(), format!("hex-{}", warm.key))
    };
    assert!(world.logged(&format!("prewarm {id} {agent} ActionShortcut")));
    // Calling again for the same window starts nothing more.
    runner.prewarm(Lane::Here, window.clone());
    let mut ask = request("q", Origin::ActionShortcut);
    ask.context.window = window;
    assert_eq!(runner.submit(ask), id);
    world.wait_until("the job", || world.jobs.lock().unwrap().len() == 1);
    assert_eq!(world.jobs.lock().unwrap()[0].agent_id, agent);
    // Another window doesn't take it.
    runner.prewarm(Lane::Here, None);
    let other = runner.submit(request("q2", Origin::Background));
    assert_ne!(other, id);
    world.release("q2");
    world.release("q");
    world.wait_for("both", |events| ended(&id)(events) && ended(&other)(events));
    world.settings.lock().unwrap().backend = AssistantBackend::Codex;
    runner.prewarm(Lane::Here, None);
    assert!(runner.state().warm.is_none());
}

#[test]
fn another_process_can_stop_every_task() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("task", Origin::Background));
    world.wait_for("the task", started(&id));
    world
        .requests
        .lock()
        .unwrap()
        .push((remote::ALL.into(), remote::Verb::Stop));
    world.wait_for("the stop", ended(&id));
}

#[test]
fn nothing_runs_while_the_assistant_is_off_or_not_set_up() {
    let world = World::new(8);
    *world.lab_on.lock().unwrap() = false;
    let runner = world.runner();
    let id = runner.submit(request("q", Origin::Chat));
    *world.lab_on.lock().unwrap() = true;
    world.settings.lock().unwrap().setup_done = false;
    let later = runner.submit(request("q", Origin::Chat));
    assert_eq!(
        world.finals(),
        [
            Event::Failed {
                conversation: id,
                message: "Turn on the assistant first.".into()
            },
            Event::Failed {
                conversation: later,
                message: "Finish setting up the assistant first (Settings › Assistant).".into()
            }
        ]
    );
    assert!(runner.tasks().is_empty());
    assert!(world.modes().is_empty());
}
