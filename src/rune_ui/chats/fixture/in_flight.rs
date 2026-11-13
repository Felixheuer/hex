//! The scenes' requests in flight: each one's turn in the pane and its
//! task in the shared view, as their events would have left them.

use super::MINUTE;
use crate::assistant::Origin;

/// The running task's turn, as the person asked it.
pub(super) fn running_turn() -> super::super::Turn {
    super::super::Turn {
        started_ms: crate::history::now_ms() - 3 * MINUTE,
        ..super::super::Turn::new(
            "Reply to Maya's last email and say Thursday at 3 works for me".into(),
            Origin::Background,
        )
    }
}

/// The running task's steps so far, as its events left them.
pub(super) const RUNNING_STEPS: [&str; 4] = [
    "Opened Mail",
    "Found Maya's email “Thursday?”",
    "Clicked Reply",
    "Typing the reply in Mail",
];

/// Puts `turn` in flight as `id`, the shared view's task doing `activity`
/// with `steps` and `answer` so far (listed tasks are in it already).
pub(super) fn in_flight(
    pane: &mut super::super::ChatsPane,
    id: &str,
    turn: super::super::Turn,
    activity: crate::assistant::view::Activity,
    steps: &[&str],
    answer: &str,
) {
    use crate::assistant::view::{Logged, Phase, Task};
    if pane.view.task(id).is_none() {
        pane.view.tasks.push(Task::new(
            id.into(),
            turn.origin,
            &turn.question,
            0,
            Phase::Running,
        ));
    }
    if let Some(task) = pane.view.task_mut(id) {
        task.activity = activity;
        task.log = steps
            .iter()
            .map(|step| Logged::Step((*step).into()))
            .collect();
        task.answer = answer.into();
    }
    pane.turns.insert(id.into(), turn);
}

/// The running task in flight, working.
pub(super) fn running(pane: &mut super::super::ChatsPane) {
    in_flight(
        pane,
        "reply-to-maya",
        running_turn(),
        crate::assistant::view::Activity::Working,
        &RUNNING_STEPS,
        "",
    );
}
