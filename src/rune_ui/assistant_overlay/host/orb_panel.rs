//! The orb's panel: its task list unfolding and folding, the keyboard
//! (Escape, the answer field), replies typed at a question, Try again,
//! and the board it keeps, which the outlines, the frame, the Hands off
//! pill, the wall, and the large card follow.

use super::*;

/// Unfolds the task list from the orb, or folds it back (a double-tap of
/// the Action shortcut). Any thread; nothing while the list would be empty
/// (no job, no result to recall). The keyboard stays where it is: the
/// person may be typing elsewhere (the global Escape still folds it).
pub(crate) fn toggle_tasks() {
    // The onboarding's tutorial checks the double-tap here.
    crate::rune_ui::assistant_tasks_opened();
    if let Some(overlays) = OVERLAYS.get() {
        overlays.orb.update(|orb, tx| {
            toggle(orb);
            // With only results to recall, the orb may be hidden.
            board_changed(orb, tx, Instant::now());
        });
    }
}

pub(super) fn toggle(orb: &mut Orb) {
    if orb.expanded || !orb::listable(&orb.board) {
        fold(orb);
        return;
    }
    orb.expanded = true;
    orb.reply = None;
    show_cards(orb);
    want_escape_list(true);
}

/// A press on the orb, a job's row, its "more" line, or the list's ×: the
/// list unfolds and takes the keyboard (Escape, and a click outside, fold
/// it), or folds.
pub(super) fn toggle_pressed(orb: &mut Orb) {
    toggle(orb);
    if orb.expanded
        && let Some(overlays) = OVERLAYS.get()
    {
        overlays.orb.request_keys();
    }
}

/// Folds the list back into the orb.
pub(super) fn fold(orb: &mut Orb) {
    orb.expanded = false;
    hover_task(orb, "", false);
    show_cards(orb);
    want_escape_list(false);
}

/// Escape while the orb has the keyboard: the answer field goes, else an
/// unfolded result folds, else the list.
pub(super) fn orb_key(orb: &mut Orb, key: &rune::KeyEvent, _tx: &mut Tx) -> bool {
    key.key == rune::Key::Escape && escape(orb)
}

/// Escape at the orb: the answer field goes, else an unfolded result folds,
/// else the list; whether anything did.
pub(in super::super) fn escape(orb: &mut Orb) -> bool {
    if orb.reply.take().is_some() {
        return true;
    }
    if let Some(open) = orb.open_result.take() {
        orb.board.pops.hold(open, false, Instant::now());
        rearm();
        return true;
    }
    if orb.expanded {
        fold(orb);
        return true;
    }
    false
}

/// The orb's panel lost the keyboard (a click outside, or another panel
/// took it): a list a press opened folds, and an empty answer field goes
/// (one with words in it stays, to finish).
pub(super) fn orb_focus(orb: &mut Orb, focused: bool, _tx: &mut Tx) {
    if focused {
        return;
    }
    if orb.expanded {
        fold(orb);
    }
    if orb
        .reply
        .as_ref()
        .is_some_and(|reply| reply.answer().is_none())
    {
        orb.reply = None;
    }
}

/// Reply on a question: its field shows and the orb takes the keyboard.
pub(super) fn start_reply(orb: &mut Orb, id: &str) {
    match orb.reply.as_mut() {
        Some(reply) if reply.id == id => reply.edit.focused = true,
        _ => orb.reply = Some(Reply::new(id)),
    }
    if let Some(overlays) = OVERLAYS.get() {
        overlays.orb.request_keys();
    }
}

pub(super) fn reply_edit(orb: &mut Orb) -> &mut TextEdit {
    &mut orb.reply.get_or_insert_with(Reply::default).edit
}

/// Return or Send: the typed answer goes to its job, as Chats' composer
/// (or the phone) would send it.
pub(super) fn send_reply(orb: &mut Orb) {
    let Some(reply) = orb.reply.take() else {
        return;
    };
    let Some(text) = reply.answer() else {
        orb.reply = Some(reply);
        return;
    };
    crate::assistant::submit(crate::assistant::Request {
        conversation: Some(reply.id),
        text,
        origin: crate::assistant::Origin::Chat,
        context: crate::assistant::Context::default(),
    });
}

/// Try again on a failed job: it runs once more in its conversation.
pub(super) fn retry(id: &str) {
    crate::assistant::submit(crate::assistant::Request {
        conversation: Some(id.to_owned()),
        text: crate::desktop::i18n::tr("That didn't work. Please try again.").to_owned(),
        // A job again, even once its task left the orb.
        origin: crate::assistant::Origin::Background,
        context: crate::assistant::Context::default(),
    });
}

/// Try again on a failed quick question: the same words once more, in its
/// conversation, as a hold of the Action shortcut asks.
pub(super) fn ask_again(id: &str, asked: &str) {
    crate::assistant::submit(crate::assistant::Request {
        conversation: Some(id.to_owned()),
        text: asked.to_owned(),
        origin: crate::assistant::Origin::ActionShortcut,
        context: crate::assistant::Context::default(),
    });
}

/// A result's bubble was held, let go, closed, or recalled: the orb wakes
/// for its new departure.
pub(super) fn rearm() {
    if let Some(overlays) = OVERLAYS.get() {
        overlays.orb.update(|orb, tx| {
            orb.armed = None;
            arm(orb, tx, Instant::now());
        });
    }
}

/// Tells the previews which jobs' rows show now, so only their browsers
/// stream (`live::cards`).
pub(super) fn show_cards(orb: &Orb) {
    let mut carded = orb::carded(&orb.board, orb.expanded);
    // The desk's minis show whenever the list is folded.
    if !orb.expanded {
        let desk = super::super::minis::now();
        if !desk.is_empty() {
            carded.push(super::super::minis::DESK.to_owned());
            carded.extend(desk.browsers);
        }
    }
    super::super::pictures::cards(&carded);
}

/// The pointer came onto task `id`'s row in the list, or left it: its
/// browsers stream live while it rests there.
pub(super) fn hover_task(orb: &mut Orb, id: &str, inside: bool) {
    let hovered = if inside {
        Some(id.to_owned())
    } else if orb.hovered.as_deref() == Some(id) || id.is_empty() {
        None
    } else {
        return;
    };
    if orb.hovered != hovered {
        orb.hovered = hovered;
        super::super::live::hover(orb.hovered.as_deref());
    }
}

pub(super) fn orb_event(orb: &mut Orb, event: &Event, tx: &mut Tx) -> bool {
    let colour = match event {
        Event::Queued { conversation, .. } | Event::Started { conversation, .. } => {
            colour_of(conversation)
        }
        _ => None,
    };
    let now = Instant::now();
    let asked_before = active_id(&orb.board);
    if !orb.board.apply(event, colour, now) {
        return false;
    }
    if let Event::QuestionsChanged { .. } = event {
        chime_for(asked_before, active_id(&orb.board));
    }
    board_changed(orb, tx, now);
    true
}

pub(super) fn active_id(board: &super::super::board::Board) -> Option<String> {
    board
        .inbox
        .as_ref()
        .and_then(|inbox| inbox.active.as_ref())
        .map(|active| active.conversation.clone())
}

/// A soft tone when another question becomes the one to answer while it
/// can't be read aloud now (something else is read, or announcements are
/// off): the bubble alone would go unnoticed.
pub(super) fn chime_for(before: Option<String>, now: Option<String>) {
    if should_chime(
        before.as_deref(),
        now.as_deref(),
        crate::assistant::speech::speaking(),
        crate::assistant::spoken_announcements(),
    ) {
        crate::feedback::play(crate::feedback::Tone::Onboarding(
            crate::feedback::OnboardingTone::PermissionTick,
        ));
    }
}

/// Whether a change of the active question from `before` to `now` chimes.
pub(in super::super) fn should_chime(
    before: Option<&str>,
    now: Option<&str>,
    speaking: bool,
    spoken: bool,
) -> bool {
    now.is_some() && now != before && (speaking || !spoken)
}

/// The service's colour for a task, read here on the main thread rather
/// than in the subscriber, which runs under the service.
pub(super) fn colour_of(conversation: &ConversationId) -> Option<u8> {
    crate::assistant::tasks()
        .into_iter()
        .find(|task| &task.conversation == conversation)
        .map(|task| task.colour)
}

pub(super) fn board_changed(orb: &mut Orb, tx: &mut Tx, now: Instant) {
    let mut outlined = orb.board.outlined();
    let hands = orb.board.hands_off().cloned();
    if let Some(task) = &hands
        && task.windows.is_empty()
    {
        // A Here task names no window of its own: the one it holds.
        outlined.extend(
            crate::assistant::guard::leases()
                .into_iter()
                .filter(|lease| lease.task == task.id)
                .map(|lease| Outlined {
                    task: task.id.clone(),
                    colour: task.colour,
                    title: task.title.clone(),
                    window: lease.window,
                    look: Look::HandsOff,
                    progress: task.progress(),
                }),
        );
    }
    let live = orb.board.tasks.iter().any(|task| task.phase.live());
    // Working tasks' browsers stream in the background from the start, so
    // their previews show at once.
    let working: Vec<String> = orb
        .board
        .tasks
        .iter()
        .filter(|task| task.phase.live())
        .map(|task| task.id.clone())
        .collect();
    // The desk's browsers stream too (quick turns' that stay).
    let desk = super::super::minis::now();
    orb.desk = desk.clone();
    let mut working = working;
    working.extend(desk.browsers.iter().cloned());
    super::super::live::track(&working);
    // Their app windows too: captured while a card shows them, and back on
    // the screen once they leave the job.
    let windows: Vec<(String, Vec<crate::assistant::TaskWindow>)> = orb
        .board
        .tasks
        .iter()
        .filter(|task| task.phase.live())
        .map(|task| (task.id.clone(), task.windows.clone()))
        .chain(std::iter::once((
            super::super::minis::DESK.to_owned(),
            desk.windows.clone(),
        )))
        .collect();
    super::super::app_live::track(&windows);
    // The frame shows only while a task works on the screen right now
    // (`Board::frame_until`): background work shows its card, not a frame.
    watcher::set_frame_until(orb.board.frame_until(now));
    watcher::publish(outlined, live);
    enlarged::follow(&orb.board);
    wall::follow(&orb.board);
    set_hands(hands.map(|task| task.id));
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    if orb.expanded && !orb::listable(&orb.board) {
        fold(orb);
    }
    // The field goes with its question (answered by voice, or another came).
    if orb.reply.as_ref().is_some_and(|reply| {
        orb.board
            .active_question()
            .is_none_or(|asking| asking.id != reply.id)
    }) {
        orb.reply = None;
    }
    if orb
        .open_result
        .is_some_and(|key| !orb.board.pops.items.iter().any(|pop| pop.key == key))
    {
        orb.open_result = None;
    }
    show_cards(orb);
    let visible = orb.board.orb_shows(now) || orb.expanded || !desk.is_empty();
    if visible && !orb.shown {
        place_orb(&overlays.orb, orb);
        overlays.orb.fade_in(FADE);
        orb.shown = true;
    } else if !visible && orb.shown {
        overlays.orb.fade_out(FADE);
        orb.shown = false;
    }
    arm(orb, tx, now);
}

/// Wakes the orb when its next finished task or bubble is due to leave, a
/// waiting task's countdown next changes, or (while its rows show live
/// pictures) each `look::LIVE_TICK`.
pub(super) fn arm(orb: &mut Orb, tx: &mut Tx, now: Instant) {
    let now_ms = when::now_ms();
    let tick = orb
        .board
        .tasks
        .iter()
        .filter_map(|task| match task.phase {
            super::super::board::Phase::Waiting { until_ms } => when::next_tick(until_ms, now_ms),
            _ => None,
        })
        .min()
        .map(|after| now + after);
    // While job rows show live pictures, look again each second, so a
    // stalled stream's mini says how long ago its last frame came.
    let live = super::super::jobs::watching(&orb.board, orb.expanded)
        .then(|| now + super::super::look::LIVE_TICK);
    let next = orb
        .board
        .next_expiry()
        .into_iter()
        .chain(tick)
        .chain(live)
        .min();
    if let Some(at) = next
        && next != orb.armed
    {
        tx.wake_after(at.saturating_duration_since(now));
    }
    orb.armed = next;
}

pub(super) fn orb_poll(orb: &mut Orb, _clock: f64, tx: &mut Tx) -> bool {
    let now = Instant::now();
    if orb.board.expire(now) {
        orb.armed = None;
        board_changed(orb, tx, now);
        return true;
    }
    if orb.armed.is_some_and(|at| now >= at) {
        // A countdown's minute, or woken a little early: look again.
        orb.armed = None;
        arm(orb, tx, now);
        return orb.board.waiting() || super::super::jobs::watching(&orb.board, orb.expanded);
    }
    false
}
