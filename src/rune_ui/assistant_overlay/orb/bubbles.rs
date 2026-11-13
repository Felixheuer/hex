//! The bubbles that stand in the rail (a task the person has, one
//! waiting out a usage limit) and how every bubble and the list grow out
//! of the orb and fold back into it.

use super::*;

/// The bubble a task keeps while the person has it, or while it waits out
/// a usage limit.
pub(super) fn standing_bubble<S: 'static>(
    p: Paint,
    task: &Task,
    grow: Grow,
    actions: stack::Actions<S>,
) -> Option<El> {
    let colour = palette::task(task.colour, p.k.dark);
    let (words, button) = match task.phase {
        Phase::Paused => {
            let hand_back = actions.hand_back;
            let id = task.id.clone();
            (
                format!("{} · {}", tr("You have it"), task.title),
                Some(
                    look::button(p, look::Kind::Primary, tr("Hand back"), move |s: &mut S| {
                        hand_back(s, &id)
                    })
                    .key("hand-back"),
                ),
            )
        }
        Phase::Waiting { until_ms } => (
            tr_fill("Waiting for limit · resumes {}", &when::clock(until_ms)),
            None,
        ),
        _ => return None,
    };
    let waiting = button.is_none();
    let bubble = shell(p, format!("standing-{}", task.id))
        .pr(if waiting { 14.0 } else { 4.0 })
        .child(dot(task, colour, 8.0))
        .child(
            text(words)
                .key("standing-text")
                .truncate()
                .font(look::label(p))
                .color(if waiting { p.muted() } else { p.text() })
                .min_w(0.0)
                .shrink(),
        )
        .children(button);
    Some(
        if waiting {
            bubble.opacity(0.75)
        } else {
            bubble
        }
        .transition(pop_in(grow)),
    )
}

/// Several tasks waiting out a usage limit, as one bubble: how many, and
/// when the first resumes.
pub(super) fn all_waiting(p: Paint, waiting: &[&Task]) -> El {
    let first = waiting
        .iter()
        .filter_map(|task| match task.phase {
            Phase::Waiting { until_ms } => Some(until_ms),
            _ => None,
        })
        .min()
        .unwrap_or_default();
    let words = format!(
        "{} · {}",
        tr_fill("{} jobs wait for the limit", &waiting.len().to_string()),
        tr_fill("resumes {}", &when::clock(first))
    );
    shell(p, "standing-waiting".into())
        .pr(14.0)
        .children(
            waiting
                .iter()
                .take(6)
                .map(|task| dot(task, palette::task(task.colour, p.k.dark), 8.0)),
        )
        .child(
            text(words)
                .key("standing-text")
                .truncate()
                .font(look::label(p))
                .color(p.muted())
                .min_w(0.0)
                .shrink(),
        )
        .opacity(0.75)
}

/// A bubble's capsule.
fn shell(p: Paint, key: String) -> El {
    look::material(
        p,
        row()
            .key(key)
            .h(BUBBLE_H)
            .max_w(BUBBLE_MAX_W)
            .pl(14.0)
            .gap(8.0)
            .center_y(),
    )
    .radius(BUBBLE_H / 2.0)
    .hit()
}

/// The point of the orb a bubble grows from, in the bubble's own unit
/// coordinates.
pub(super) fn anchor(grow: Grow) -> Vec2 {
    Vec2::new(
        if grow.right { 0.0 } else { 1.0 },
        if grow.down { 0.0 } else { 1.0 },
    )
}

/// A bubble pops out of the orb with a little overshoot, and dissolves.
pub(super) fn pop_in(grow: Grow) -> Transition {
    let toward = if grow.down { -10.0 } else { 10.0 };
    Transition::asymmetric(
        Transition::scale_anchor(0.55, anchor(grow))
            .allow_small_scale()
            .combined(Transition::offset(0.0, toward)),
        Transition::scale_anchor(0.8, anchor(grow))
            .allow_small_scale()
            .combined(Transition::opacity()),
    )
    .springs(rune::SpringPair::new(
        Spring::new(0.42, 0.32),
        Spring::new(0.28, 0.0),
    ))
}

/// The list unfolds from the orb and folds back into it.
pub(super) fn unfold(grow: Grow) -> Transition {
    Transition::scale_anchor(0.86, anchor(grow))
        .allow_small_scale()
        .combined(Transition::offset(
            0.0,
            if grow.down { -12.0 } else { 12.0 },
        ))
        .springs(rune::SpringPair::new(
            Spring::new(0.38, 0.18),
            Spring::new(0.26, 0.0),
        ))
}
