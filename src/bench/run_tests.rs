//! The runner's tests: a fake app, desktop, and clock.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::check::{ToolCall, Win};
use super::results::{Status, TaskResult};
use super::run::{self, App, Clock, Conv, Desktop, Options};
use super::task::{Config, Lang, Task};
use super::tests::{QUICK, parse};

#[derive(Clone)]
struct FakeClock(Rc<Cell<u64>>);

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }

    fn sleep_ms(&self, ms: u64) {
        self.0.set(self.0.get() + ms);
    }
}

/// How the fake assistant answers a text.
#[derive(Clone, Default)]
struct Behavior {
    answer: String,
    answer_after: u64,
    end_after: u64,
    outcome: &'static str,
    origin: &'static str,
    tools: Vec<ToolCall>,
    /// A job the turn starts, ending this long after asking.
    child_ends_after: Option<u64>,
    /// Until this long after asking, the conversation still shows the
    /// request before (the app hasn't picked it up yet).
    start_after: u64,
}

struct Submission {
    id: String,
    at: u64,
    behavior: Behavior,
}

struct FakeApp {
    now: Rc<Cell<u64>>,
    behaviors: Vec<(&'static str, Behavior)>,
    submitted: Vec<Submission>,
    stopped: Vec<String>,
    asked: Vec<(String, Option<String>)>,
}

impl FakeApp {
    fn new(now: Rc<Cell<u64>>, behaviors: Vec<(&'static str, Behavior)>) -> Self {
        Self {
            now,
            behaviors,
            submitted: Vec::new(),
            stopped: Vec::new(),
            asked: Vec::new(),
        }
    }
}

impl App for FakeApp {
    fn version(&mut self) -> color_eyre::Result<String> {
        Ok("1.2.3".into())
    }

    fn submit(&mut self, text: &str, conversation: Option<&str>) -> color_eyre::Result<String> {
        let behavior = self
            .behaviors
            .iter()
            .find(|(when, _)| text.contains(when))
            .map(|(_, behavior)| behavior.clone())
            .unwrap_or_default();
        let id =
            conversation.map_or_else(|| format!("conv-{}", self.submitted.len()), str::to_owned);
        self.asked
            .push((text.to_owned(), conversation.map(str::to_owned)));
        self.submitted.push(Submission {
            id: id.clone(),
            at: self.now.get(),
            behavior,
        });
        Ok(id)
    }

    fn conversation(&mut self, id: &str) -> Option<Conv> {
        let now = self.now.get();
        if let Some(parent) = id.strip_prefix("child-") {
            let submission = self.submitted.iter().rev().find(|s| s.id == parent)?;
            let ends = submission.at + submission.behavior.child_ends_after?;
            return Some(Conv {
                outcome: if now < ends { "running" } else { "answered" }.into(),
                origin: "background".into(),
                workspace: Some(PathBuf::from("/nonexistent/job")),
                ..Conv::default()
            });
        }
        let mut conv = Conv::default();
        let mut found = false;
        for submission in self.submitted.iter().filter(|s| s.id == id) {
            let behavior = &submission.behavior;
            if now < submission.at + behavior.start_after {
                continue;
            }
            found = true;
            conv.origin = behavior.origin.into();
            if now >= submission.at + behavior.answer_after && !behavior.answer.is_empty() {
                conv.replies.push(submission.at + behavior.answer_after);
                conv.answers.push((
                    submission.at + behavior.answer_after,
                    behavior.answer.clone(),
                ));
            }
            conv.outcome = if now >= submission.at + behavior.end_after {
                behavior.outcome.into()
            } else {
                "running".into()
            };
        }
        found.then_some(conv)
    }

    fn created_since(&mut self, since_ms: u64) -> Vec<String> {
        self.submitted
            .iter()
            .filter(|s| s.at >= since_ms && s.behavior.child_ends_after.is_some())
            .filter(|s| self.now.get() >= s.at + 100)
            .map(|s| format!("child-{}", s.id))
            .collect()
    }

    fn tool_calls(&mut self, id: &str) -> Vec<ToolCall> {
        self.submitted
            .iter()
            .filter(|s| s.id == id)
            .flat_map(|s| s.behavior.tools.clone())
            .collect()
    }

    fn stop(&mut self, id: &str) {
        self.stopped.push(id.to_owned());
    }
}

struct FakeDesktop {
    now: Rc<Cell<u64>>,
    /// The front app from this time on.
    fronts: Vec<(u64, &'static str)>,
    apps: Vec<String>,
    windows: BTreeMap<String, Vec<Win>>,
}

impl FakeDesktop {
    fn still(now: Rc<Cell<u64>>) -> Self {
        Self {
            now,
            fronts: vec![(0, "Google Chrome")],
            apps: vec!["Finder".into(), "Google Chrome".into()],
            windows: BTreeMap::new(),
        }
    }
}

impl Desktop for FakeDesktop {
    fn front_app(&mut self) -> Option<String> {
        let now = self.now.get();
        self.fronts
            .iter()
            .rev()
            .find(|(from, _)| *from <= now)
            .map(|(_, app)| (*app).to_owned())
    }

    fn apps(&mut self) -> Option<Vec<String>> {
        Some(self.apps.clone())
    }

    fn windows(&mut self, app: &str) -> Option<Vec<Win>> {
        Some(self.windows.get(app).cloned().unwrap_or_default())
    }
}

static RUN_DIRS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn options(langs: Vec<Lang>) -> Options {
    Options {
        langs,
        allow_front: false,
        poll_ms: 100,
        // A folder per call: tests run side by side.
        run_dir: std::env::temp_dir().join(format!(
            "hex-bench-run-{}-{}",
            std::process::id(),
            RUN_DIRS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )),
        run_id: "run1".into(),
        background_wait_ms: 2000,
    }
}

fn answered(answer: &str, after: u64) -> Behavior {
    Behavior {
        answer: answer.into(),
        answer_after: after,
        end_after: after + 50,
        outcome: "answered",
        origin: "action_shortcut",
        ..Behavior::default()
    }
}

fn run_tasks(
    tasks: &[Task],
    app: &mut FakeApp,
    desktop: &mut FakeDesktop,
    clock: &FakeClock,
    options: &Options,
    config: &Config,
) -> Vec<TaskResult> {
    run::run(tasks, config, app, desktop, clock, options, &mut |_| {})
}

#[test]
fn a_quick_task_is_timed_and_passes() {
    let now = Rc::new(Cell::new(1_000_000));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![("17", answered("Das ist 391.", 1200))]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(QUICK)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De, Lang::En]),
        &Config::new(),
    );
    assert_eq!(results.len(), 2);
    let first = &results[0];
    assert_eq!(first.status, Status::Passed, "{}", first.reason);
    assert_eq!(first.first_answer_ms, Some(1200));
    assert!(first.total_ms.unwrap() >= 1250);
    assert_eq!(first.lane.as_deref(), Some("quick"));
    assert_eq!(app.asked[0].0, "Wie viel ist 17 mal 23?");
    assert_eq!(app.asked[1].0, "What is 17 times 23?");
    assert!(app.stopped.is_empty());
}

#[test]
fn a_task_that_runs_too_long_times_out_and_is_stopped() {
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let slow = Behavior {
        end_after: 10_000_000,
        outcome: "answered",
        ..Behavior::default()
    };
    let mut app = FakeApp::new(now.clone(), vec![("17", slow)]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(QUICK)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Timeout);
    assert_eq!(app.stopped, vec!["conv-0".to_owned()]);
    assert!(results[0].total_ms.unwrap() >= 30_000);
}

#[test]
fn a_follow_up_continues_the_earlier_conversation() {
    let follow = r#"
id = "follow"
title = "Follow"
timeout_s = 30
after = "quick"
[text]
de = "Und geteilt durch 3?"
en = "And divided by 3?"
[[check]]
kind = "answer_matches"
pattern = "130"
[[check]]
kind = "same_conversation"
"#;
    let now = Rc::new(Cell::new(5_000));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(
        now.clone(),
        vec![
            ("17", answered("391", 500)),
            // The app shows the earlier answer's outcome for a moment
            // before the follow-up runs: that isn't the follow-up's end.
            (
                "durch",
                Behavior {
                    start_after: 400,
                    ..answered("Etwa 130,33.", 700)
                },
            ),
        ],
    );
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(QUICK), parse(follow)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[1].status, Status::Passed, "{}", results[1].reason);
    assert_eq!(app.asked[1].1.as_deref(), Some("conv-0"));
    // Only what came after asking is the follow-up's answer.
    assert!(!results[1].answer.contains("391"));
    assert_eq!(results[1].first_answer_ms, Some(700));
}

#[test]
fn a_follow_up_whose_task_was_skipped_is_skipped() {
    let follow = QUICK
        .replace("id = \"quick\"", "id = \"follow\"")
        .replace("lane = \"quick\"", "after = \"quick\"");
    let skipped_first = format!("{QUICK}[safety]\nfront = \"may_change\"\n");
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(&skipped_first), parse(&follow)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Skipped);
    assert!(results[0].reason.contains("--allow-front"));
    assert_eq!(results[1].status, Status::Skipped);
    assert!(app.asked.is_empty(), "nothing is asked");
}

#[test]
fn a_task_needing_config_is_skipped_not_guessed() {
    let draft = r#"
id = "draft"
title = "Draft"
timeout_s = 60
[safety]
messages = "draft"
[text]
de = "Schreib {test_contact} einen Entwurf"
en = "Draft a message to {test_contact}"
[[check]]
kind = "tool_not_used"
pattern = "send"
"#;
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(draft)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Skipped);
    assert!(results[0].reason.contains("test_contact"));
    assert!(app.asked.is_empty());
}

#[test]
fn taking_the_front_window_fails_a_keep_task() {
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![("17", answered("391", 1000))]);
    let mut desktop = FakeDesktop::still(now.clone());
    desktop.fronts.push((500, "TextEdit"));
    let results = run_tasks(
        &[parse(QUICK)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Failed);
    assert!(
        results[0].reason.contains("front app"),
        "{}",
        results[0].reason
    );
}

#[test]
fn a_question_during_a_job_is_answered_while_it_runs() {
    let during = r#"
id = "during"
title = "During"
lane = "quick"
timeout_s = 30
[background]
de = "Recherchiere lange"
en = "Research for long"
lead_ms = 1000
[text]
de = "Wie viel ist 12 mal 12?"
en = "What is 12 times 12?"
[[check]]
kind = "answer_matches"
pattern = "144"
[[check]]
kind = "while_running"
"#;
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let job = Behavior {
        answer: "Bericht fertig".into(),
        answer_after: 60_000,
        end_after: 60_000,
        outcome: "answered",
        origin: "background",
        ..Behavior::default()
    };
    let mut app = FakeApp::new(
        now.clone(),
        vec![("Recherchiere", job), ("12 mal", answered("144", 800))],
    );
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(during)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Passed, "{}", results[0].reason);
    // The job is asked first, the question at least its lead later, and the
    // bench stops the job afterwards.
    assert_eq!(app.asked[0].0, "Recherchiere lange");
    assert!(app.submitted[1].at >= app.submitted[0].at + 1000);
    assert_eq!(app.stopped, vec!["conv-0".to_owned()]);
}

#[test]
fn jobs_a_turn_starts_are_waited_for_and_make_it_a_job() {
    let job_task = QUICK
        .replace("lane = \"quick\"", "lane = \"job\"")
        .replace("timeout_s = 30", "timeout_s = 120");
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let starts_job = Behavior {
        child_ends_after: Some(20_000),
        tools: vec![ToolCall {
            tool: "hex_start_jobs".into(),
            input: "{}".into(),
        }],
        ..answered("Ich starte 391 als Job.", 900)
    };
    let mut app = FakeApp::new(now.clone(), vec![("17", starts_job)]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(&job_task)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    let result = &results[0];
    assert_eq!(result.status, Status::Passed, "{}", result.reason);
    assert_eq!(result.lane.as_deref(), Some("job"));
    assert!(result.total_ms.unwrap() >= 20_000, "{:?}", result.total_ms);
    assert_eq!(result.steps, Some(1));
}

#[test]
fn preconditions_skip_a_task_the_mac_isnt_set_up_for() {
    let needs_chrome =
        format!("{QUICK}[[precondition]]\nkind = \"app_running\"\napp = \"Numbers\"\n");
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![]);
    let mut desktop = FakeDesktop::still(now.clone());
    let results = run_tasks(
        &[parse(&needs_chrome)],
        &mut app,
        &mut desktop,
        &clock,
        &options(vec![Lang::De]),
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Skipped);
    assert!(
        results[0].reason.contains("Numbers runs"),
        "{}",
        results[0].reason
    );
}

#[test]
fn seed_files_are_written_with_placeholders_filled() {
    let seeded = r#"
id = "seeded"
title = "Seeded"
timeout_s = 30
[[seed]]
name = "find-{run}.txt"
content = "hello"
[text]
de = "Finde find-{run}.txt"
en = "Find find-{run}.txt"
[[check]]
kind = "file"
glob = "find-run1.txt"
"#;
    let now = Rc::new(Cell::new(0));
    let clock = FakeClock(now.clone());
    let mut app = FakeApp::new(now.clone(), vec![("find", answered("gefunden", 300))]);
    let mut desktop = FakeDesktop::still(now.clone());
    let options = options(vec![Lang::En]);
    let results = run_tasks(
        &[parse(seeded)],
        &mut app,
        &mut desktop,
        &clock,
        &options,
        &Config::new(),
    );
    assert_eq!(results[0].status, Status::Passed, "{}", results[0].reason);
    assert_eq!(app.asked[0].0, "Find find-run1.txt");
    assert!(options.run_dir.join("seeded-en/find-run1.txt").is_file());
    std::fs::remove_dir_all(&options.run_dir).unwrap();
}
