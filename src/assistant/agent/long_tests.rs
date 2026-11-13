//! A long job, simulated: twelve steps of 55 minutes each (eleven hours)
//! on a fake backend whose session grows with every step, so Agent-Desk
//! rolls it over to fresh sessions several times; the network goes away
//! once; Agent-Desk crashes once and the job resumes. A fresh session
//! knows only what its first message tells it (as a real one would know
//! only that and what it reads), so a seed without the plan would redo
//! work. Nothing may be done twice, and the journal must tell the story in
//! order.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::assistant::types::Context;

const STEPS: usize = 12;
const STEP_MS: u64 = 55 * 60_000;
const STEP_TOKENS: u64 = 45_000;
const START_MS: u64 = 1_794_472_200_000;

#[derive(Default)]
struct World {
    clock: u64,
    /// The network is away until then.
    online_at: u64,
    /// What each session knows is done.
    sessions: HashMap<String, (BTreeSet<usize>, u64)>,
    made: u32,
    /// Every step done, in order (twice if redone).
    done: Vec<usize>,
    /// Each main run: (the session it resumed, its first message).
    runs: Vec<(Option<String>, String)>,
    checks: u32,
    crash_after: Option<usize>,
    offline_before: Option<usize>,
}

struct Fake {
    world: Mutex<World>,
}

impl Fake {
    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|error| error.into_inner())
    }
}

fn plan_line(step: usize, done: bool) -> String {
    format!(
        "- [{}] Step {step} (files) — check: `out-{step}.txt`",
        if done { "x" } else { " " }
    )
}

/// The steps a message says are ticked.
fn ticked(prompt: &str) -> BTreeSet<usize> {
    prompt
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("- [x] Step ")?;
            rest.split_whitespace().next()?.parse().ok()
        })
        .collect()
}

fn tick(workspace: &Path, step: usize) {
    let path = workspace.join(workspace::PLAN);
    let plan = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        plan.replace(&plan_line(step, false), &plan_line(step, true)),
    )
    .unwrap();
}

impl Engine for Fake {
    fn run(
        &self,
        run: &Run,
        control: &Control,
        report: &mut dyn FnMut(Report),
    ) -> color_eyre::Result<provider::Finished> {
        let workspace = run.workspace.clone().expect("a job has its workspace");
        if run.prompt.contains("independent checker") {
            self.world().checks += 1;
            let missing: Vec<usize> = (1..=STEPS)
                .filter(|step| !workspace.join(format!("out-{step}.txt")).exists())
                .collect();
            let text = if missing.is_empty() {
                "Every file is there.\nHEX_VERIFIED".to_owned()
            } else {
                format!("HEX_NOT_DONE: missing steps {missing:?}")
            };
            return Ok(provider::Finished { text });
        }
        let rotations = control.rotations();
        let (session, mut known, mut tokens) = {
            let mut world = self.world();
            world.runs.push((run.resume.clone(), run.prompt.clone()));
            match &run.resume {
                Some(session) => {
                    let Some((known, tokens)) = world.sessions.get(session).cloned() else {
                        return Err(color_eyre::eyre::eyre!(
                            "Claude Code: No conversation found with session ID: {session}"
                        ));
                    };
                    (session.clone(), known, tokens)
                }
                None => {
                    world.made += 1;
                    let session = format!("s{}", world.made);
                    // A fresh session knows only what it was told.
                    let known = ticked(&run.prompt);
                    world
                        .sessions
                        .insert(session.clone(), (known.clone(), 10_000));
                    (session, known, 10_000)
                }
            }
        };
        report(Report::Session(session.clone()));
        let plan = workspace.join(workspace::PLAN);
        if !plan.exists() {
            let lines: Vec<String> = (1..=STEPS).map(|step| plan_line(step, false)).collect();
            std::fs::write(&plan, format!("# Plan\n{}\n", lines.join("\n"))).unwrap();
        }
        for step in 1..=STEPS {
            if known.contains(&step) {
                continue;
            }
            {
                let mut world = self.world();
                if world.offline_before == Some(step) {
                    world.offline_before = None;
                    world.online_at = world.clock + 40 * 60_000;
                    return Err(color_eyre::eyre::eyre!(
                        "Claude Code: API Error: Connection error. (ECONNRESET)"
                    ));
                }
                world.clock += STEP_MS;
                world.done.push(step);
            }
            std::fs::write(workspace.join(format!("out-{step}.txt")), "done").unwrap();
            tick(&workspace, step);
            if step % 2 == 0 {
                let mut journal = std::fs::OpenOptions::new()
                    .append(true)
                    .open(workspace.join(journal::JOURNAL))
                    .unwrap();
                std::io::Write::write_all(
                    &mut journal,
                    format!(
                        "- learned: step {step} needs the files of step {}\n",
                        step - 1
                    )
                    .as_bytes(),
                )
                .unwrap();
            }
            known.insert(step);
            tokens += STEP_TOKENS;
            let crash = {
                let mut world = self.world();
                world
                    .sessions
                    .insert(session.clone(), (known.clone(), tokens));
                (world.crash_after == Some(step)).then(|| world.crash_after = None)
            };
            if crash.is_some() {
                panic!("Agent-Desk crashed (simulated)");
            }
            report(Report::Usage(rollover::Usage {
                context_tokens: tokens,
            }));
            if control.rotations() > rotations {
                // Agent-Desk ended the process between two steps.
                return Err(color_eyre::eyre::eyre!(
                    "Claude Code stopped with exit code 143 and no answer"
                ));
            }
        }
        Ok(provider::Finished {
            text: format!("All {STEPS} steps are done: out-1.txt to out-{STEPS}.txt."),
        })
    }

    fn now_ms(&self) -> u64 {
        START_MS + self.world().clock
    }

    fn sleep(&self, duration: Duration) {
        self.world().clock += duration.as_millis() as u64;
    }

    fn online(&self) -> bool {
        let world = self.world();
        world.clock >= world.online_at
    }
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hex-long-{name}-{}-{}",
        std::process::id(),
        store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn long_job(workspace: &Path) -> Job {
    let settings = AssistantSettings {
        control_desktop: false,
        background_browser: false,
        memory: false,
        job_budget_minutes: 14 * 60,
        ..AssistantSettings::default()
    };
    Job {
        conversation: format!("long-job-{}", std::process::id()),
        request: Request {
            conversation: None,
            text: "Do the twelve steps and keep a file of each".into(),
            origin: Origin::Background,
            context: Context::default(),
        },
        settings,
        kind: JobKind::Job {
            title: "Twelve steps".into(),
        },
        digest: None,
        workspace: Some(workspace.to_path_buf()),
        authorized: Default::default(),
        memory: None,
        recall: None,
        helps: None,
    }
}

/// The `hex:` lines of the journal, without their time.
fn hex_lines(journal: &str) -> Vec<String> {
    journal
        .lines()
        .filter_map(|line| line.split_once(" hex: ").map(|(_, rest)| rest.to_owned()))
        .collect()
}

#[test]
fn a_long_job_rolls_over_survives_a_crash_and_the_network_and_redoes_nothing() {
    let root = folder("job");
    let workspace = root.join("2026-11-12 Twelve steps");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = Store::at(root.join("conversations"));
    let fake = Fake {
        world: Mutex::new(World {
            crash_after: Some(7),
            offline_before: Some(4),
            ..World::default()
        }),
    };
    let job = long_job(&workspace);

    // It works, rolls over, loses the network, and then Agent-Desk crashes.
    let control = Control::new();
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_with(&fake, &store, &job, &control, &mut |_| {})
    }));
    assert!(crashed.is_err(), "the simulated crash ends the first run");
    assert_eq!(fake.world().done, (1..=7).collect::<Vec<_>>());

    // The restart: the job resumes (as `service::resume_interrupted` does
    // for a conversation an Agent-Desk that quit left running).
    let mut resumed = job.clone();
    resumed.kind = JobKind::Resume;
    let outcome = run_with(&fake, &store, &resumed, &Control::new(), &mut |_| {});
    let Outcome::Answer(answer) = outcome else {
        panic!("the job finishes: {outcome:?}");
    };
    assert!(answer.contains("All 12 steps"), "{answer}");

    let world = fake.world();
    // Nothing was done twice, nothing left out, in order, over 11 hours.
    assert_eq!(world.done, (1..=STEPS).collect::<Vec<_>>());
    assert!(world.clock >= 10 * 3_600_000, "{} ms", world.clock);
    assert_eq!(world.checks, 1, "checked once, at the end");
    // Several fresh sessions, each seeded with the plan and the journal.
    let fresh: Vec<String> = world
        .runs
        .iter()
        .skip(1)
        .filter(|(resumed, _)| resumed.is_none())
        .map(|(_, prompt)| prompt.clone())
        .collect();
    assert!(fresh.len() >= 3, "{} fresh sessions", fresh.len());
    for prompt in &fresh {
        assert!(prompt.contains("in a fresh session"), "{prompt}");
        assert!(prompt.contains("- [x] Step 1 (files)"), "{prompt}");
        assert!(prompt.contains("journal.md (your memory"), "{prompt}");
        assert!(prompt.contains("Time: running for"), "{prompt}");
        assert!(prompt.contains("Do the twelve steps"), "{prompt}");
    }
    assert!(
        fresh.last().unwrap().contains("learned: step 10"),
        "the agent's own lines go on"
    );
    // After the network and the crash the job went on in its own session,
    // told where it was.
    let went_on: Vec<&String> = world
        .runs
        .iter()
        .filter(|(resumed, _)| resumed.is_some())
        .map(|(_, prompt)| prompt)
        .collect();
    assert!(
        went_on
            .iter()
            .any(|prompt| prompt.contains("The network went away"))
    );
    assert!(
        went_on
            .iter()
            .any(|prompt| prompt.starts_with(RESUME_NOTE) || prompt.contains(RESUME_NOTE))
    );
    drop(world);

    // The journal tells it in order, each thing once.
    let text = std::fs::read_to_string(workspace.join(journal::JOURNAL)).unwrap();
    let lines = hex_lines(&text);
    let at = |wanted: &str| {
        let found: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with(wanted))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(found.len(), 1, "“{wanted}” once in:\n{text}");
        found[0]
    };
    assert_eq!(at("started:"), 0);
    let milestones: Vec<usize> = (1..=STEPS)
        .map(|step| at(&format!("milestone: Step {step} (files)")))
        .collect();
    assert!(
        milestones.windows(2).all(|pair| pair[0] < pair[1]),
        "{text}"
    );
    let offline = at("waiting: for the network");
    let back = at("back: the network is back");
    assert!(milestones[2] < offline && offline < back && back < milestones[3]);
    let restarted = at("resumed: after Agent-Desk restarted");
    assert!(
        milestones[5] < restarted && restarted < milestones[7],
        "{text}"
    );
    let sessions: Vec<u32> = lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("new session: #")?;
            rest.split(',').next()?.parse().ok()
        })
        .collect();
    assert_eq!(sessions, (2..2 + fresh.len() as u32).collect::<Vec<_>>());
    assert!(
        !lines.iter().any(|line| line.starts_with("check failed")),
        "{text}"
    );
    assert_eq!(at("checked: verified") + 1, lines.len() - 1);
    assert_eq!(lines.last().map(String::as_str), Some("done: answered"));
    let state: journal::State =
        serde_json::from_slice(&std::fs::read(workspace.join(journal::STATE)).unwrap()).unwrap();
    assert_eq!(state.started_ms, START_MS);
    assert_eq!(state.sessions, 1 + fresh.len() as u32);
    assert_eq!(state.milestones.len(), STEPS);
    // The conversation ended answered, with the last session saved.
    let stored = store.load(&job.conversation).unwrap();
    assert_eq!(stored.current_outcome(), Ended::Answered);
    let _ = std::fs::remove_dir_all(&root);
}

/// A step ticked without the file its check names: Agent-Desk notes it and
/// the job's next instruction says so.
#[test]
fn a_ticked_step_without_its_file_comes_back_to_the_job() {
    let root = folder("check");
    let workspace = root.join("job");
    std::fs::create_dir_all(&workspace).unwrap();
    let job = long_job(&workspace);
    let mut long = long::Long::open(&job, &workspace, rollover::Limits::default(), START_MS);
    std::fs::write(
        workspace.join(workspace::PLAN),
        "- [x] Build the sheet (files) — check: `suppliers.xlsx` has 50 rows\n- [ ] Draft emails\n",
    )
    .unwrap();
    let reminder = long.reminder(START_MS + 3 * 3_600_000);
    assert!(reminder.contains("Time: running for 3 h"), "{reminder}");
    assert!(reminder.contains("suppliers.xlsx"), "{reminder}");
    assert!(reminder.contains("another way"), "{reminder}");
    // Said once.
    assert!(
        !long
            .reminder(START_MS + 4 * 3_600_000)
            .contains("suppliers.xlsx")
    );
    let text = long.journal.text();
    assert!(text.contains("hex: check failed:"), "{text}");
    assert_eq!(journal::failures(&text, 5).len(), 1);
    // Nearly out of time: it wraps up.
    assert!(
        long.reminder(START_MS + 13 * 3_600_000)
            .contains(rollover::NEARLY_OUT)
    );
    let _ = std::fs::remove_dir_all(&root);
}
