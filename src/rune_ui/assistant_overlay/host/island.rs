//! The island's part of the panels: while the Action shortcut is held for
//! a quick turn, the bubble's panel shows what is heard as it is
//! recognised (`island::held_card`), fed by the dictation's live text,
//! which the hold asks for (`rune_ui::live_text::island_words`); the
//! release hands over to the question's own bubble. The reasoning folds to
//! Now and Next on time (`quick::Bubble::fold`).

use std::time::Instant;

use rune::Tx;

use super::super::bubble::BubbleOverlay;
use super::super::frame::Tab;
use super::super::quick::Held;
use super::answer::{hide_bubble, show_bubble};
use super::{OVERLAYS, answer_key};

/// Hears the dictation's live text: while the shortcut is held, its words
/// go to the held card. Any thread; it only queues the update.
pub(super) fn install() {
    crate::dictation::live_text::watch(|partial| {
        let Some(partial) = partial.cloned() else {
            return;
        };
        let Some(overlays) = OVERLAYS.get() else {
            return;
        };
        overlays.bubble.update_if(move |state, _| {
            state
                .held
                .as_mut()
                .is_some_and(|held| held.hear(&partial.stable, &partial.tentative))
        });
    });
}

/// The shortcut is held for a quick turn about `app`: the card under the
/// island listens. Main thread.
pub(super) fn hold(app: Option<String>) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.bubble.update(move |state, _| {
        state.held = Some(Held::new(app));
        state.setup = None;
        // A finished answer gives way; a question waiting stays for the
        // turn this hold continues.
        if state.bubble.as_ref().is_some_and(|bubble| bubble.over()) {
            state.bubble = None;
        }
        state.answer_key = answer_key();
        if !state.shown
            && let Some(overlays) = OVERLAYS.get()
        {
            show_bubble(state, &overlays.bubble);
        }
    });
}

/// The hold ended (sent, discarded, or cancelled): the held card goes; the
/// question's bubble takes its place once it starts. Main thread.
pub(super) fn release() {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.bubble.update_if(|state, _| {
        if state.held.take().is_none() {
            return false;
        }
        if state.bubble.is_none() && state.setup.is_none() {
            hide_bubble(state);
        }
        true
    });
}

/// The front's moment as the board says it now (`frame::Tab::of`): a
/// window brought in front, or the front just given back. The island says
/// it, showing its panel for it if nothing else does; real input stays the
/// frame tab's alone. Main thread.
pub(super) fn front(tab: Option<Tab>) {
    let tab = tab.filter(|tab| !matches!(tab, Tab::Hands { .. }));
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.bubble.update_if(move |state, tx| {
        if state.front == tab {
            return false;
        }
        state.front = tab;
        settle_front(state, tx, Instant::now());
        true
    });
}

/// Shows the panel while the island has the front's moment to say, wakes
/// it when "Back to you" is due to go, and hides a panel that has nothing
/// else; whether it changed.
pub(super) fn settle_front(state: &mut BubbleOverlay, tx: &mut Tx, now: Instant) -> bool {
    let mut changed = false;
    if let Some(tab) = &state.front {
        if tab.shows(now) {
            if let Tab::Back { until, .. } = tab {
                tx.wake_after(until.saturating_duration_since(now));
            }
        } else {
            state.front = None;
            changed = true;
        }
    }
    let alone = state.bubble.is_none() && state.held.is_none() && state.setup.is_none();
    match (&state.front, state.shown) {
        (Some(_), false) => {
            if let Some(overlays) = OVERLAYS.get() {
                show_bubble(state, &overlays.bubble);
            }
            true
        }
        (None, true) if alone => {
            hide_bubble(state);
            true
        }
        _ => changed,
    }
}

/// Folds the bubble's reasoning when its time comes, and wakes the panel
/// for it; whether it changed.
pub(super) fn fold(state: &mut BubbleOverlay, tx: &mut Tx, now: Instant) -> bool {
    let Some(bubble) = state.bubble.as_mut() else {
        return false;
    };
    let (folded, next) = bubble.fold(now);
    if let Some(at) = next {
        tx.wake_after(at.saturating_duration_since(now));
    }
    folded
}
