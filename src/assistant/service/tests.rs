//! The runner with a fake backend: queueing, parallel slots, cancelling,
//! Agent mode's session, follow-ups, and speech.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::assistant::types::{Context, TaskWindow};

/// A fake world: tasks wait until their question is released or they are
/// cancelled; everything the runner did is recorded.
#[derive(Default)]
struct World {
    released: Mutex<HashSet<String>>,
    events: Mutex<Vec<Event>>,
    sessions: Mutex<Vec<String>>,
    jobs: Mutex<Vec<Job>>,
    spoken: Mutex<Vec<String>>,
    lab_on: Mutex<bool>,
    settings: Mutex<AssistantSettings>,
}

impl World {
    fn new(max_parallel: u32) -> Arc<Self> {
        let world = Arc::new(Self::default());
        *world.lab_on.lock().unwrap() = true;
        world.settings.lock().unwrap().max_parallel = max_parallel;
        world
    }

    fn runner(self: &Arc<Self>) -> Arc<Runner> {
        let execute = Arc::clone(self);
        let settings = Arc::clone(self);
        let start = Arc::clone(self);
        let end = Arc::clone(self);
        let style = Arc::clone(self);
        let speak = Arc::clone(self);
        let runner = Runner::new(Hooks {
            execute: Box::new(move |job, control, emit| {
                execute.jobs.lock().unwrap().push(job.clone());
                emit(Event::Step {
                    conversation: job.conversation.clone(),
                    description: "Looking at Mail".into(),
                });
                emit(Event::Window {
                    conversation: job.conversation.clone(),
                    window: TaskWindow {
                        app: "Mail".into(),
                        window_id: None,
                    },
                });
                loop {
                    if control.cancelled() {
                        return Outcome::Cancelled;
                    }
                    if execute.released.lock().unwrap().contains(&job.request.text) {
                        return Outcome::Answer(format!("answer to {}", job.request.text));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }),
            settings: Box::new(move || {
                (
                    *settings.lab_on.lock().unwrap(),
                    settings.settings.lock().unwrap().clone(),
                )
            }),
            start_session: Box::new(move || {
                start.sessions.lock().unwrap().push("start".into());
                Some("S1".into())
            }),
            end_session: Box::new(move |id| end.sessions.lock().unwrap().push(format!("end {id}"))),
            style_agent: Box::new(move |session, agent, colour| {
                style
                    .sessions
                    .lock()
                    .unwrap()
                    .push(format!("style {session} {agent} {colour}"));
            }),
            speak: Box::new(move |text, _| speak.spoken.lock().unwrap().push(text.to_owned())),
            stop_speaking: Box::new(|| {}),
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

    fn wait_for(&self, what: &str, done: impl Fn(&[Event]) -> bool) {
        let started = Instant::now();
        while !done(&self.events.lock().unwrap()) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}: {:#?}",
                self.events.lock().unwrap()
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
fn tasks_beyond_max_parallel_wait_their_turn() {
    let world = World::new(2);
    let runner = world.runner();
    let a = runner.submit(request("a", Origin::Chat));
    let b = runner.submit(request("b", Origin::Background));
    let c = runner.submit(request("c", Origin::Chat));
    world.wait_for("a and b to start", |events| {
        started(&a)(events) && started(&b)(events)
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
            (c.clone(), TaskState::Queued)
        ]
    );
    let colours: Vec<u8> = runner.tasks().iter().map(|task| task.colour).collect();
    assert_eq!(colours, [0, 1, 2]);
    world.wait_for("a's step", |_| {
        runner.tasks()[0].step.as_deref() == Some("Looking at Mail")
            && runner.tasks()[0].windows.len() == 1
    });
    assert!(world.events.lock().unwrap().iter().any(
        |event| matches!(event, Event::Queued { conversation, question, .. } if *conversation == c && question == "c")
    ));
    assert!(!started(&c)(&world.events.lock().unwrap()));

    world.release("b");
    world.wait_for("c to start once b is done", started(&c));
    // One at a time: released together, a and c may answer in either order.
    world.release("a");
    world.wait_for("a's answer", ended(&a));
    world.release("c");
    world.wait_for("every answer", |events| {
        ended(&a)(events) && ended(&c)(events)
    });
    assert!(runner.tasks().is_empty());
    assert_eq!(
        world.finals(),
        [
            Event::Answer {
                conversation: b,
                origin: Origin::Background,
                text: "answer to b".into()
            },
            Event::Answer {
                conversation: a,
                origin: Origin::Chat,
                text: "answer to a".into()
            },
            Event::Answer {
                conversation: c,
                origin: Origin::Chat,
                text: "answer to c".into()
            },
        ]
    );
}

#[test]
fn a_new_quick_question_cancels_the_last_one_but_not_agent_tasks() {
    let world = World::new(8);
    let runner = world.runner();
    let agent = runner.submit(request("agent", Origin::Background));
    let first = runner.submit(request("first", Origin::ActionShortcut));
    world.wait_for("the first quick question", started(&first));
    let second = runner.submit(request("second", Origin::ActionShortcut));
    world.wait_for("the first to be cancelled", ended(&first));
    world.wait_for("the second to start", started(&second));
    assert_eq!(
        world.finals(),
        [Event::Cancelled {
            conversation: first
        }]
    );

    world.release("second");
    world.wait_for("the second answer", ended(&second));
    assert_eq!(*world.spoken.lock().unwrap(), ["answer to second"]);
    assert_eq!(runner.tasks().len(), 1);
    assert_eq!(runner.tasks()[0].conversation, agent);
    runner.cancel(&agent);
    world.wait_for("the agent task to stop", ended(&agent));
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
    world.release("a");
    world.wait_for("a's answer", ended(&a));
    assert!(!started(&b)(&world.events.lock().unwrap()));
}

#[test]
fn a_follow_up_waits_for_its_conversation() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("one", Origin::Chat));
    let mut follow_up = request("two", Origin::Chat);
    follow_up.conversation = Some(id.clone());
    assert_eq!(runner.submit(follow_up), id);
    world.wait_for("the first to start", started(&id));
    assert_eq!(runner.tasks()[1].state, TaskState::Queued);
    world.release("one");
    world.release("two");
    world.wait_for("both answers", |events| {
        events
            .iter()
            .filter(|event| matches!(event, Event::Answer { .. }))
            .count()
            == 2
    });
}

#[test]
fn agent_mode_runs_tasks_in_one_desktop_session_until_off_and_idle() {
    let world = World::new(8);
    let runner = world.runner();
    runner.set_agent_mode(true);
    let task = runner.submit(request("task", Origin::Background));
    world.wait_for("the task", started(&task));
    world.wait_for("the job", |_| !world.jobs.lock().unwrap().is_empty());
    let job = world.jobs.lock().unwrap()[0].clone();
    assert_eq!(job.desktop_session.as_deref(), Some("S1"));
    assert!(runner.focus(30).is_none());

    runner.set_agent_mode(false);
    assert_eq!(
        *world.sessions.lock().unwrap(),
        ["start".to_owned(), format!("style S1 {} 0", job.agent_id)]
    );
    world.release("task");
    world.wait_for("the answer", ended(&task));
    assert_eq!(world.sessions.lock().unwrap().last().unwrap(), "end S1");
    let modes: Vec<_> = world
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            Event::ModeChanged { on } => Some(*on),
            _ => None,
        })
        .collect();
    assert_eq!(modes, [true, false]);
}

#[test]
fn a_quick_answer_is_followed_up_within_the_window() {
    let world = World::new(8);
    let runner = world.runner();
    let id = runner.submit(request("q", Origin::ActionShortcut));
    world.release("q");
    world.wait_for("the answer", ended(&id));
    assert_eq!(runner.focus(30), Some(id.clone()));
    assert_eq!(runner.focus(0), None);
    runner.state().focus = Some("chat".into());
    assert_eq!(runner.focus(30).as_deref(), Some("chat"));
}

#[test]
fn nothing_runs_while_the_lab_is_off() {
    let world = World::new(8);
    *world.lab_on.lock().unwrap() = false;
    let runner = world.runner();
    let id = runner.submit(request("q", Origin::Chat));
    assert_eq!(
        world.finals(),
        [Event::Failed {
            conversation: id,
            message: "Turn on Labs › Assistant first.".into()
        }]
    );
    runner.set_agent_mode(true);
    assert!(!runner.state().agent_mode);
    assert!(runner.tasks().is_empty());
}
