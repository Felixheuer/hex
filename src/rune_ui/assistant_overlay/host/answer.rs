//! The Here question's bubble (and "Finish setting up" in its place) past
//! the HUD, and the global Esc that dismisses it and stops a task holding
//! the real pointer.

use std::time::{Duration, Instant};

use rune::{OverlayHandle, Tx};

use super::super::board::{Bubble, Due};
use super::super::bubble::{self, BubbleOverlay};
use super::super::hands;
use super::super::screens;
use super::follow;
use super::{FADE, OVERLAYS, answer_key, hands_task};
use crate::assistant::Event;

/// The bubble's panel fades a little slower, so its card can shrink back.
const BUBBLE_OUT: Duration = Duration::from_millis(260);
/// How long "Finish setting up the assistant first" stays.
const SETUP_SHOWS: Duration = Duration::from_secs(7);
/// How often the pulse follows a measured voice.
const VOICE_TICK: Duration = Duration::from_millis(33);

pub(super) fn bubble_event(state: &mut BubbleOverlay, event: &Event, tx: &mut Tx) -> bool {
    let now = Instant::now();
    let before = state.bubble.clone();
    if !Bubble::apply(&mut state.bubble, event, now) {
        return false;
    }
    let Some(overlays) = OVERLAYS.get() else {
        return true;
    };
    match &state.bubble {
        None => {
            // Cancelled: it shrinks away with what it showed.
            state.bubble = before;
            hide_bubble(state);
        }
        Some(bubble) => {
            let fresh = before
                .as_ref()
                .is_none_or(|old| old.conversation != bubble.conversation);
            state.setup = None;
            if bubble.escalated {
                // Drawn once flying, then gone toward the orb.
                tx.wake_after(Duration::from_millis(30));
                return true;
            }
            if fresh || !state.shown {
                state.answer_key = answer_key();
                show_bubble(state, &overlays.bubble);
            }
            settle_bubble(state, tx, now);
        }
    }
    true
}

fn show_bubble(state: &mut BubbleOverlay, handle: &OverlayHandle<BubbleOverlay>) {
    state.below = place_bubble(handle);
    handle.fade_in(FADE);
    state.shown = true;
    want_escape(Some(true), None);
}

/// "Finish setting up the assistant first", with Continue setup, where the
/// Here bubble shows: the trigger calls this when the Action shortcut is
/// used before the assistant is set up. Any thread; it leaves by itself.
pub(crate) fn setup_needed() {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    overlays.bubble.update(|state, tx| {
        state.bubble = None;
        state.setup = Some(Instant::now() + SETUP_SHOWS);
        if let Some(overlays) = OVERLAYS.get() {
            show_bubble(state, &overlays.bubble);
        }
        tx.wake_after(SETUP_SHOWS);
    });
}

pub(super) fn bubble_poll(state: &mut BubbleOverlay, _clock: f64, tx: &mut Tx) -> bool {
    settle_bubble(state, tx, Instant::now())
}

/// Hides an answered bubble once its time is up, and follows its speech;
/// whether it changed.
fn settle_bubble(state: &mut BubbleOverlay, tx: &mut Tx, now: Instant) -> bool {
    if !state.shown {
        return false;
    }
    if let Some(until) = state.setup {
        if now >= until {
            hide_bubble(state);
            return true;
        }
        tx.wake_after(until - now);
        return false;
    }
    let speaking = crate::assistant::speech::speaking();
    let level = speaking
        .then(crate::assistant::speech::level)
        .flatten()
        .map(bubble::pulse);
    let voice_changed = state.speaking != speaking || state.level != level;
    state.speaking = speaking;
    if state.level != level {
        state.level = level;
        // The orb pulses with the same voice.
        if let Some(overlays) = OVERLAYS.get() {
            overlays.orb.update_if(move |orb, _| {
                let changed = orb.voice != level;
                orb.voice = level;
                changed && orb.shown
            });
        }
    }
    if level.is_some() {
        // Follow the voice: about 30 times a second while it's measured.
        tx.wake_after(VOICE_TICK);
    }
    let Some(bubble) = state.bubble.as_mut() else {
        return voice_changed;
    };
    match bubble.due(now, speaking) {
        Due::Stay => voice_changed,
        Due::At(at) => {
            tx.wake_after(at.saturating_duration_since(now));
            voice_changed
        }
        Due::Leave => {
            hide_bubble(state);
            true
        }
    }
}

fn hide_bubble(state: &mut BubbleOverlay) {
    if !state.shown {
        return;
    }
    state.shown = false;
    state.speaking = false;
    want_escape(Some(false), None);
    if let Some(overlays) = OVERLAYS.get() {
        overlays.bubble.fade_out(BUBBLE_OUT);
    }
}

/// A press on the card, or Esc: the bubble goes and stops reading aloud.
/// The question itself goes on; its answer is in Chats.
pub(super) fn dismiss_bubble(state: &mut BubbleOverlay) {
    if state.shown && state.setup.is_none() {
        crate::assistant::speech::stop();
    }
    hide_bubble(state);
}

/// Places the bubble where Live text would show on the display the person
/// works on, just past the HUD's plate (and past the Hands off pill while
/// it shows); whether it hangs below that spot.
pub(super) fn place_bubble(handle: &OverlayHandle<BubbleOverlay>) -> bool {
    let inset = f64::from(bubble::INSET);
    let room = if hands_task().is_some() {
        f64::from(hands::SIZE.1)
    } else {
        0.0
    };
    let Some((screen, all)) = follow::working() else {
        #[cfg(not(target_os = "macos"))]
        return follow::by_hud(handle, (room - inset) as f32, rune::Align::Center);
        #[cfg(target_os = "macos")]
        return true;
    };
    let (width, height) = (f64::from(bubble::SIZE.0), f64::from(bubble::SIZE.1));
    let (x, y, below) = follow::hud_spot(&screen, &all);
    let top = if below {
        y - inset + room
    } else {
        y - height + inset - room
    };
    let rect = screens::snap([x - width / 2.0, top, width, height], screen.scale);
    follow::set_rect(handle, rect.map(|v| v as f32));
    below
}

// ---------------------------------------------------------------------------
// Esc

thread_local! {
    /// Which of the bubble and the Hands off pill want Esc.
    static ESCAPE_WANTED: std::cell::Cell<(bool, bool)> =
        const { std::cell::Cell::new((false, false)) };
}

/// Notes whether the bubble and the pill want Esc, and watches for it while
/// either does (`native::watch_escape`).
pub(super) fn want_escape(bubble: Option<bool>, hands: Option<bool>) {
    let (was_bubble, was_hands) = ESCAPE_WANTED.get();
    let wanted = (bubble.unwrap_or(was_bubble), hands.unwrap_or(was_hands));
    ESCAPE_WANTED.set(wanted);
    super::native::watch_escape(wanted.0 || wanted.1);
}

/// Esc, pressed by the person while the bubble or the pill showed: stops
/// the task moving the real pointer and dismisses the bubble. macOS's
/// passive monitor calls it; elsewhere the listener holds Esc.
#[cfg(target_os = "macos")]
pub(super) fn escape_pressed(by_person: bool) {
    if let Some(task) = hands_task()
        && by_person
    {
        crate::assistant::cancel(&task);
    }
    if let Some(overlays) = OVERLAYS.get() {
        overlays.bubble.update_if(|state, _| {
            dismiss_bubble(state);
            false
        });
    }
}
