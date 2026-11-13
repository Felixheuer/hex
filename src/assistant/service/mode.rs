//! What Background work changes beside itself: the computer stays awake
//! while jobs work, and a task taken over brings its window forward. The
//! screen frame is the overlay's own (`Board::frame_until`).

use std::sync::Arc;

use super::{Lane, Runner};
use crate::assistant::types::TaskWindow;

impl Runner {
    /// Keeps the computer awake while Background jobs work (one waiting for
    /// the person's answer doesn't).
    pub(super) fn refresh_mode(self: &Arc<Self>) {
        let (_, settings) = (self.hooks.settings)();
        let working = self
            .state()
            .tasks
            .iter()
            .filter(|task| {
                task.lane() == Lane::Background
                    && task.status.state != crate::assistant::TaskState::NeedsInput
            })
            .count();
        crate::desktop::keep_awake::update(
            settings.keep_awake,
            settings.keep_display_awake,
            working,
        );
    }
}

/// Brings the window `task` holds (its lease, else the last window it
/// named) forward: the person asked to take it over, so this one physical
/// change is theirs.
pub(super) fn bring_forward(task: &str, windows: &[TaskWindow]) {
    let leased: Vec<TaskWindow> = crate::assistant::guard::leases()
        .into_iter()
        .filter(|lease| lease.task == task)
        .map(|lease| lease.window)
        .collect();
    let window = leased
        .iter()
        .chain(windows.iter().rev())
        .find(|window| window.window_id.is_some())
        .or_else(|| leased.first().or(windows.last()))
        .cloned();
    let Some(window) = window else {
        return;
    };
    std::thread::spawn(move || {
        if crate::assistant::space::show_now(&window).is_err() {
            tracing::warn!(app = window.app, "couldn't bring the task's window forward");
        }
    });
}
