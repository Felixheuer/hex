//! What the runner watches while tasks run: each task's time limit, the
//! guard's "Hands off" for Here tasks, requests from other Hex processes,
//! taking a task over and handing it back, and waiting out a usage limit.

use std::sync::Arc;
use std::time::Duration;

use super::remote::{self, Verb};
use super::{Lane, Runner, Task, TaskState};
use crate::assistant::provider::Control;
use crate::assistant::types::Event;

/// How often the monitor looks.
const TICK: Duration = Duration::from_millis(250);

impl Runner {
    /// Runs until no task is left.
    pub(super) fn monitor(self: Arc<Self>) {
        loop {
            (self.hooks.sleep)(TICK);
            if !self.tick() {
                return;
            }
        }
    }

    /// One look; false once no task is left (the monitor ends).
    pub(super) fn tick(self: &Arc<Self>) -> bool {
        let (_, settings) = (self.hooks.settings)();
        let now = self.now();
        let mut expired: Vec<Arc<Control>> = Vec::new();
        let here: Vec<String> = {
            let mut state = self.state();
            if state.tasks.is_empty() {
                state.monitor = false;
                return false;
            }
            for task in state.tasks.iter_mut().filter(|task| task.running) {
                if task.status.state == TaskState::Running
                    && task.deadline_ms.is_some_and(|deadline| now >= deadline)
                {
                    task.deadline_ms = None;
                    expired.push(Arc::clone(&task.control));
                }
            }
            state
                .tasks
                .iter()
                .filter(|task| task.running && task.lane() == Lane::Here)
                .map(|task| task.status.conversation.clone())
                .collect()
        };
        let minutes = settings.task_timeout_minutes;
        for control in expired {
            control.fail(&format!(
                "Stopped after {minutes} minutes: a task may run at most {minutes} minutes (Settings › Assistant)."
            ));
        }
        self.check_hands_off(&here);
        self.check_plans();
        for (target, verb) in (self.hooks.requests)() {
            let ours = target == remote::ALL
                || self
                    .state()
                    .tasks
                    .iter()
                    .any(|task| task.status.conversation == target);
            if !ours {
                continue;
            }
            match (verb, target == remote::ALL) {
                (Verb::Stop, true) => self.cancel_all(),
                (Verb::Stop, false) => self.cancel(&target),
                (Verb::TakeOver, false) => self.take_over(&target),
                (Verb::HandBack, false) => self.hand_back(&target),
                (_, true) => {}
            }
        }
        true
    }

    /// Emits `Plan` when a job's `plan.md` changes.
    fn check_plans(self: &Arc<Self>) {
        let jobs: Vec<(u64, std::path::PathBuf, Option<std::time::SystemTime>)> = self
            .state()
            .tasks
            .iter()
            .filter_map(|task| Some((task.key, task.workspace.clone()?, task.plan_seen)))
            .collect();
        for (key, workspace, seen) in jobs {
            let path = workspace.join(crate::assistant::agent::workspace::PLAN);
            let Ok(modified) = std::fs::metadata(&path).and_then(|meta| meta.modified()) else {
                continue;
            };
            if seen == Some(modified) {
                continue;
            }
            let steps = crate::assistant::agent::workspace::plan(&workspace);
            let changed = {
                let mut state = self.state();
                let Some(task) = state.tasks.iter_mut().find(|task| task.key == key) else {
                    continue;
                };
                task.plan_seen = Some(modified);
                (task.plan != steps).then(|| {
                    task.plan = steps.clone();
                    task.status.conversation.clone()
                })
            };
            if let Some(conversation) = changed {
                self.emit(&Event::Plan {
                    conversation,
                    steps,
                });
            }
        }
    }

    /// Emits `HandsOff` as the guard's answer changes while Here tasks run.
    fn check_hands_off(self: &Arc<Self>, here: &[String]) {
        let now = if here.is_empty() {
            None
        } else {
            (self.hooks.hands_off)().filter(|task| here.contains(task))
        };
        let before = {
            let mut state = self.state();
            if state.hands_off == now {
                return;
            }
            std::mem::replace(&mut state.hands_off, now.clone())
        };
        if let Some(conversation) = before {
            self.emit(&Event::HandsOff {
                conversation,
                on: false,
            });
        }
        if let Some(conversation) = now {
            self.emit(&Event::HandsOff {
                conversation,
                on: true,
            });
        }
    }

    pub(crate) fn take_over(self: &Arc<Self>, conversation: &str) {
        let now = self.now();
        let windows = {
            let mut state = self.state();
            let Some(task) = state.tasks.iter_mut().find(|task| {
                task.running
                    && task.status.conversation == conversation
                    && task.status.state != TaskState::Paused
            }) else {
                return;
            };
            task.before_pause = task.status.state;
            task.status.state = TaskState::Paused;
            task.held_since.get_or_insert(now);
            task.status.windows.clone()
        };
        (self.hooks.pause)(conversation, true);
        self.emit(&Event::Paused {
            conversation: conversation.to_owned(),
        });
        (self.hooks.bring_forward)(conversation, &windows);
    }

    pub(crate) fn hand_back(self: &Arc<Self>, conversation: &str) {
        let now = self.now();
        {
            let mut state = self.state();
            let Some(task) = state.tasks.iter_mut().find(|task| {
                task.status.conversation == conversation && task.status.state == TaskState::Paused
            }) else {
                return;
            };
            task.status.state = task.before_pause;
            if task.status.state == TaskState::Running {
                resume_clock(task, now);
            }
        }
        (self.hooks.pause)(conversation, false);
        self.emit(&Event::Resumed {
            conversation: conversation.to_owned(),
        });
    }

    /// Waits until `until` (or until the task is stopped) after a usage
    /// limit: `TaskState::Waiting`, `Event::Waiting`, then running again.
    pub(super) fn wait_out(
        self: &Arc<Self>,
        key: u64,
        conversation: &str,
        control: &Control,
        until: u64,
        reason: String,
    ) {
        let now = self.now();
        if let Some(task) = self.state().tasks.iter_mut().find(|task| task.key == key) {
            if task.status.state == TaskState::Paused {
                task.before_pause = TaskState::Waiting { until_ms: until };
            } else {
                task.status.state = TaskState::Waiting { until_ms: until };
            }
            task.held_since.get_or_insert(now);
        }
        tracing::info!(
            conversation,
            until,
            reason,
            "an assistant task waits out a limit"
        );
        self.emit(&Event::Waiting {
            conversation: conversation.to_owned(),
            until_ms: until,
            reason,
        });
        loop {
            let now = self.now();
            if now >= until || control.cancelled() {
                break;
            }
            (self.hooks.sleep)(Duration::from_millis((until - now).min(1000)));
        }
        let now = self.now();
        if let Some(task) = self.state().tasks.iter_mut().find(|task| task.key == key) {
            match task.status.state {
                TaskState::Paused => task.before_pause = TaskState::Running,
                _ => {
                    task.status.state = TaskState::Running;
                    resume_clock(task, now);
                }
            }
        }
    }
}

/// Moves `task`'s time limit on by the time it was held (paused, waiting,
/// or asking).
pub(super) fn resume_clock(task: &mut Task, now: u64) {
    if let Some(since) = task.held_since.take()
        && let Some(deadline) = task.deadline_ms.as_mut()
    {
        *deadline += now.saturating_sub(since);
    }
}
