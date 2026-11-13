//! The orb itself: its glowing core, one dot per live task (a ring of
//! progress round a plan's), and its badge of open questions.

use super::*;

/// The orb: its glowing core, a dot per live task, how many questions wait
/// on the person, and, with nothing live, how many results it recalls.
pub(super) fn orb<S: 'static>(
    p: Paint,
    tasks: &[Task],
    done: usize,
    questions: usize,
    look: &Look,
    actions: Actions<S>,
) -> El {
    let c = &p.k.c;
    let (lead, trail) = palette::frame(p.k.dark);
    let core = el()
        .key("core")
        .size(CORE, CORE)
        .radius(CORE / 2.0)
        .gradient(lead, trail)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 10.0,
            spread: 1.0,
            color: lead.alpha(0.6),
            inset: false,
        })
        .a11y_hidden();
    let live_tasks: Vec<&Task> = tasks.iter().filter(|task| task.phase.live()).collect();
    let core = match look.voice {
        // With the voice reading an answer aloud.
        Some(level) => core.opacity(0.5 + 0.5 * super::super::bubble::pulse(level)),
        // Nothing at work: still, and quieter.
        None if look.still || live_tasks.is_empty() => core,
        None => core.looping(LoopSpec::opacity(frame::BREATH, 0.0, 0.62, 1.0)),
    };
    // The newest few as dots; the rest as a count, so fifty jobs keep the
    // orb small.
    let hidden = live_tasks.len().saturating_sub(MAX_DOTS);
    let dots = live_tasks[hidden..].iter().map(|task| {
        let colour = palette::task(task.colour, p.k.dark);
        marker(p, task, colour, DOT)
            .key(format!("orb-dot-{}", task.id))
            .transition(fly_in(look.grow))
    });
    let mut items: Vec<El> = vec![core];
    items.extend((hidden > 0).then(|| {
        text(format!("+{hidden}"))
            .key("orb-more")
            .font(p.k.ty.small)
            .color(p.faint())
    }));
    items.extend(dots);
    // Nothing live: what it can recall, a press away.
    let recalling = live_tasks.is_empty() && done > 0;
    items.extend(recalling.then(|| {
        row()
            .key("orb-recent")
            .gap(3.0)
            .center_y()
            .child(
                icon(p.k.glyph.check_small, 12.0)
                    .color(p.muted())
                    .a11y_hidden(),
            )
            .child(
                text(done.to_string())
                    .font(p.k.ty.small_medium)
                    .color(p.muted()),
            )
    }));
    items.extend((questions > 0).then(|| badge(p, questions, look.grow)));
    if !look.grow.right {
        items.reverse();
    }
    let toggle = actions.list.toggle;
    let drag = actions.drag;
    let live = live_tasks.len();
    let mut label = tr_fill("Assistant tasks: {}", &live.to_string());
    if questions > 0 {
        label = format!(
            "{label} · {}",
            tr_fill("Open questions: {}", &questions.to_string())
        );
    }
    if recalling {
        label = format!(
            "{label} · {}",
            tr_fill("Recent results: {}", &done.to_string())
        );
    }
    let pill = row()
        .key("orb")
        .h(ORB_H)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(ORB_H / 2.0)
        .fill(c.bg_base.alpha(0.94))
        .border(p.hair, c.border_base)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: lead.alpha(if p.k.dark { 0.35 } else { 0.22 }),
            inset: false,
        })
        .role("button")
        .label(label)
        .hit()
        .on_press(move |s: &mut S| toggle(s))
        .drag_threshold(4.0)
        .on_drag(move |s: &mut S, d: &mut rune::Drag, _| drag(s, d))
        .children(items);
    if live == 0 && questions == 0 {
        pill.opacity(0.72)
    } else {
        pill
    }
}

/// A task's dot in `colour`: full while it runs, a ring while the person
/// has it, faint while it waits or queues, soft once it's over.
pub(crate) fn dot(task: &Task, colour: rune::Color, size: f32) -> El {
    let base = el().size(size, size).radius(size / 2.0).a11y_hidden();
    match task.phase {
        Phase::Running => base.fill(colour).box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 6.0,
            spread: 0.0,
            color: colour.alpha(0.55),
            inset: false,
        }),
        Phase::Paused => base.border(1.5, colour),
        Phase::NeedsInput => base.fill(colour.alpha(0.3)).border(1.5, colour),
        Phase::Waiting { .. } => base.fill(colour.alpha(0.35)),
        Phase::Queued => base.fill(colour.alpha(0.45)),
        Phase::Done | Phase::Failed | Phase::Stopped => base.fill(colour.alpha(0.4)),
    }
}

/// A task's dot, inside a ring that fills as its plan's steps get done.
pub(crate) fn marker(p: Paint, task: &Task, colour: rune::Color, size: f32) -> El {
    let Some((done, total)) = task.progress() else {
        return el().center().child(dot(task, colour, size));
    };
    el().size(RING, RING)
        .center()
        .label(format!("{done}/{total}"))
        .child(
            canvas(ring(done as f32 / total as f32, colour, p.k.c.border_base))
                .key("ring")
                .absolute(0.0, 0.0)
                .size(RING, RING)
                .a11y_hidden(),
        )
        .child(dot(task, colour, size - 2.0))
}

/// A ring `RING` points across: a faint track, and `fraction` of it in
/// `colour` clockwise from the top.
fn ring(fraction: f32, colour: rune::Color, track: rune::Color) -> Shape {
    let r = (RING - RING_W) / 2.0;
    let c = RING / 2.0;
    let point = |t: f32| {
        let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        [c + r * a.cos(), c + r * a.sin()]
    };
    const SEGMENTS: usize = 48;
    let track_points: Vec<[f32; 2]> = (0..SEGMENTS)
        .map(|i| point(i as f32 / SEGMENTS as f32))
        .collect();
    let shape = Shape::new().stroke_with(track_points, true, RING_W, LineCap::Round, track);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return shape;
    }
    let steps = ((SEGMENTS as f32 * fraction).ceil() as usize).max(2);
    let arc: Vec<[f32; 2]> = (0..=steps)
        .map(|i| point(fraction * i as f32 / steps as f32))
        .collect();
    shape.stroke_with(arc, false, RING_W, LineCap::Round, colour)
}

/// The open questions' count on the orb, in the accent colour; it pops
/// again whenever the count changes (a new question arrived).
fn badge(p: Paint, count: usize, grow: Grow) -> El {
    let accent = p.accent();
    text(count.to_string())
        .key(format!("questions-{count}"))
        .font(p.k.ty.small_medium)
        .color(p.k.c.bg_base)
        .min_w(16.0)
        .h(16.0)
        .px(4.0)
        .radius(8.0)
        .fill(accent)
        .align(rune::Align::Center)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 8.0,
            spread: 0.0,
            color: accent.alpha(0.55),
            inset: false,
        })
        .a11y_hidden()
        .transition(
            Transition::scale_anchor(0.4, anchor(grow))
                .allow_small_scale()
                .combined(Transition::opacity())
                .springs(rune::SpringPair::new(
                    Spring::new(0.42, 0.32),
                    Spring::new(0.28, 0.0),
                )),
        )
}
