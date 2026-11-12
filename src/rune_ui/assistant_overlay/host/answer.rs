//! The Here question's bubble (and "Finish setting up" in its place) past
//! the HUD, and the global Esc that dismisses it and stops a task holding
//! the real pointer.

use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};
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
/// Esc's key code.
const ESCAPE: u16 = 53;

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
    let voice_changed = state.speaking != speaking;
    state.speaking = speaking;
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
    let Some((screen, all)) = follow::working() else {
        return true;
    };
    let (width, height) = (f64::from(bubble::SIZE.0), f64::from(bubble::SIZE.1));
    let inset = f64::from(bubble::INSET);
    let room = if hands_task().is_some() {
        f64::from(hands::SIZE.1)
    } else {
        0.0
    };
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
    /// The global Esc monitor while the bubble or the Hands off pill shows,
    /// and which of them wants it.
    static ESCAPE_MONITOR: std::cell::RefCell<Option<Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
    static ESCAPE_WANTED: std::cell::Cell<(bool, bool)> =
        const { std::cell::Cell::new((false, false)) };
}

/// Notes whether the bubble and the pill want Esc, and watches for it while
/// either does.
pub(super) fn want_escape(bubble: Option<bool>, hands: Option<bool>) {
    let (was_bubble, was_hands) = ESCAPE_WANTED.get();
    let wanted = (bubble.unwrap_or(was_bubble), hands.unwrap_or(was_hands));
    ESCAPE_WANTED.set(wanted);
    watch_escape(wanted.0 || wanted.1);
}

/// Watches for Esc in any app (a passive monitor: the key still reaches the
/// app). Esc stops the task moving the real pointer, unless that task
/// pressed it itself, and dismisses the bubble. Main thread only.
fn watch_escape(on: bool) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    ESCAPE_MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        match (on, slot.take()) {
            (true, Some(monitor)) => *slot = Some(monitor),
            (true, None) => {
                let block = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
                    // SAFETY: AppKit passes a live event for the handler's duration.
                    let event = unsafe { event.as_ref() };
                    if event.keyCode() != ESCAPE {
                        return;
                    }
                    if let Some(task) = hands_task()
                        && pressed_by_person(event)
                    {
                        crate::assistant::cancel(&task);
                    }
                    if let Some(overlays) = OVERLAYS.get() {
                        overlays.bubble.update_if(|state, _| {
                            dismiss_bubble(state);
                            false
                        });
                    }
                });
                // AppKit retains the block; the monitor is removed on this
                // (main) thread when nothing wants it.
                *slot = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown,
                    &block,
                );
            }
            (false, Some(monitor)) => {
                // SAFETY: the monitor came from addGlobalMonitor on this thread.
                unsafe { NSEvent::removeMonitor(&monitor) };
            }
            (false, None) => {}
        }
    });
}

/// `kCGEventSourceUnixProcessID`: the process that posted an event.
const SOURCE_PID: u32 = 41;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventGetIntegerValueField(event: *mut std::ffi::c_void, field: u32) -> i64;
}

/// Whether a key came from the person rather than from the agent pressing
/// keys for its task (agent-desktop, or Hex itself).
fn pressed_by_person(event: &NSEvent) -> bool {
    // SAFETY: `CGEvent` returns the event's own CGEventRef (or null), alive
    // while the event is; reading a field doesn't retain or change it.
    let pid = unsafe {
        let cg: *mut std::ffi::c_void = objc2::msg_send![event, CGEvent];
        if cg.is_null() {
            return true;
        }
        CGEventGetIntegerValueField(cg, SOURCE_PID)
    };
    if pid <= 0 {
        return true;
    }
    if pid == i64::from(std::process::id()) {
        return false;
    }
    let mut name = [0u8; 256];
    // SAFETY: `proc_name` writes at most `name.len()` bytes into the buffer.
    let written =
        unsafe { libc::proc_name(pid as i32, name.as_mut_ptr().cast(), name.len() as u32) };
    let name = std::str::from_utf8(&name[..written.max(0) as usize]).unwrap_or_default();
    !name.starts_with("agent-desktop")
}
