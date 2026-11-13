//! Background first, as Codex works: a task acts in its windows where they
//! are (parked off the screen, or behind the person's), never bringing one
//! forward just to act. Only when an app ignores background actions (one
//! failed, and failed again on the retry), or keys must go to it, does
//! `desktop_focus_window` bring the window forward, briefly: a parked
//! window comes on the screen, the next few steps run in it while the
//! frame shows, and then the guard gives the front back to the person's
//! app and parks the window again, by itself.
//!
//! A focus asked before any background action failed in that window is
//! refused once with how to work without it; asked again, it goes ahead.
//!
//! The borrow is stored a moment ([`ANNOUNCE`]) before the window comes
//! forward, so the island announces it first; a take-over meanwhile
//! cancels it. While a window is borrowed the guard says so in the store
//! (`guard/borrowed/<task>.json`: the window, why, until when, and the
//! person's app that gets the front back), so Agent-Desk shows the desk
//! frame with its tab and "Back to you" after (`Event::Borrowed`,
//! `Event::Returned`). When the person takes the task over meanwhile (any
//! touch does), the window is theirs: it stays where it is, in front.

use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use super::super::child::{Desktop, Window};
use super::super::state::BorrowFile;
use super::{Guard, Resolved, refusal};

/// How long a borrowed front lasts at most.
const BORROW_FOR: Duration = Duration::from_secs(20);
/// Acting steps in the borrowed window before the front goes back.
const BORROW_STEPS: u32 = 4;
/// How long the island says "Agent-Desk is using … for a moment" before
/// the window comes forward, so it doesn't jump in front unannounced
/// (Agent-Desk reads the borrow every 250 ms).
pub(super) const ANNOUNCE: Duration = Duration::from_millis(if cfg!(test) { 0 } else { 400 });
/// Failed background actions in a window after which focusing is offered.
pub(super) const FAILS_BEFORE_FOCUS: u32 = 2;

/// The window brought forward, and what to put back.
pub(super) struct Borrowed {
    /// The person's app in front before (pid).
    before: Option<i32>,
    window: Window,
    /// It hung off the screen (parked) before: parked again after.
    parked: bool,
    at: Instant,
    steps: u32,
}

/// What a refused first focus says.
pub(super) const BACKGROUND_FIRST: &str = "Background first: act in the window where it is (click, type, set_value, select, scroll \
     by ref work in a parked or covered window) instead of bringing it in front of the person. \
     If this app ignored a background action and its retry, or keys (desktop_press) must reach \
     it, call desktop_focus_window again: Agent-Desk then brings it forward briefly and gives the \
     person's window back right after your next steps.";

impl<D: Desktop> Guard<D> {
    /// `desktop_focus_window` on a window this task may use (checked).
    pub(super) fn focus(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
    ) -> Result<Value, Value> {
        let Some(target) = resolved.first().cloned() else {
            return self.forward(tool, args);
        };
        let Some(id) = target.id.clone() else {
            return self.forward(tool, args);
        };
        if target.focused {
            // In front already: nothing moves.
            return self.forward(tool, args);
        }
        let failed = self.failures.get(&id).copied().unwrap_or(0) >= FAILS_BEFORE_FOCUS;
        if !failed && self.focus_asked.insert(id.clone()) {
            return Err(refusal(tool, "BACKGROUND_FIRST", BACKGROUND_FIRST));
        }
        // An earlier borrow ends first.
        self.restore_borrowed();
        let windows = self.list(super::Look::Check).unwrap_or_default();
        let listed = windows.iter().find(|window| window.id == id).cloned();
        let window = listed.unwrap_or(Window {
            id: id.clone(),
            app: target.app.clone(),
            title: String::new(),
            pid: None,
            focused: false,
            visible: true,
            minimized: false,
            bounds: None,
        });
        let displays = self.screen.displays();
        let parked = window
            .bounds
            .is_some_and(|bounds| hangs_off(bounds, &displays));
        let before = super::front::frontmost();
        let before_app = before.and_then(|pid| {
            windows
                .iter()
                .find(|window| window.pid == Some(i64::from(pid)))
                .map(|window| window.app.clone())
        });
        // Announce first: the borrow is stored (the island says so and the
        // frame comes up), and the window comes forward a moment later.
        // It begins when it comes forward, so a touch during the announce
        // isn't counted against the person.
        let at_ms = crate::desktop::events::now_ms() + self.announce.as_millis() as u64;
        self.store.set_borrowed(
            &self.config.task,
            Some(&BorrowFile {
                task: self.config.task.clone(),
                pid: self.pid,
                started: super::super::state::started_us(self.pid),
                window_id: window.id.clone(),
                app: window.app.clone(),
                why: reason(failed).to_owned(),
                at_ms,
                until_ms: at_ms + BORROW_FOR.as_millis() as u64,
                before: before_app,
            }),
        );
        std::thread::sleep(self.announce);
        if self.store.paused(&self.config.task) {
            // Taken over during the announce: nothing comes forward; the
            // person has the task (and the window, where it is).
            self.store.set_borrowed(&self.config.task, None);
            return Err(super::resumed_refusal(tool));
        }
        if parked && let Some(spot) = on_screen_spot(&displays) {
            // Where it was before it was parked, when that still shows.
            let spot = self.unparked_spot(&window, spot);
            self.move_window(&window, spot);
        }
        let result = match self.forward(tool, args) {
            Ok(result) => result,
            Err(error) => {
                self.store.set_borrowed(&self.config.task, None);
                return Err(error);
            }
        };
        self.borrowed = Some(Borrowed {
            before,
            window: window.clone(),
            parked,
            at: Instant::now(),
            steps: 0,
        });
        self.notes.push(format!(
            "{id} is in front briefly; do the step it needs now. Agent-Desk gives the person's window \
             back after your next {BORROW_STEPS} steps in it (or {} s){}.",
            BORROW_FOR.as_secs(),
            if parked { " and parks it again" } else { "" }
        ));
        Ok(result)
    }

    /// Before a call: a borrow the person took over is theirs; one that
    /// ran out of time ends.
    pub(super) fn borrow_expired(&mut self) {
        if self.borrowed.is_some() && self.store.paused(&self.config.task) {
            self.hand_over_borrowed();
            return;
        }
        if self
            .borrowed
            .as_ref()
            .is_some_and(|borrowed| borrowed.at.elapsed() >= BORROW_FOR)
        {
            self.restore_borrowed();
        }
    }

    /// An acting call on `resolved`: one in another window ends the borrow
    /// first; one in the borrowed window counts as one of its steps.
    pub(super) fn borrow_step(&mut self, resolved: &[Resolved]) {
        let Some(borrowed) = self.borrowed.as_mut() else {
            return;
        };
        let inside = resolved
            .iter()
            .all(|window| window.id.as_deref() == Some(borrowed.window.id.as_str()));
        if !inside {
            self.restore_borrowed();
            return;
        }
        borrowed.steps += 1;
    }

    /// After an acting call: the borrow's last step ends it.
    pub(super) fn borrow_done(&mut self) {
        if self
            .borrowed
            .as_ref()
            .is_some_and(|borrowed| borrowed.steps >= BORROW_STEPS)
        {
            self.restore_borrowed();
        }
    }

    /// The person took the task over while its window was in front: the
    /// window is theirs now, left where it is (never parked away from
    /// under their hands), and the borrow is over.
    pub(super) fn hand_over_borrowed(&mut self) {
        if self.borrowed.take().is_some() {
            self.store.set_borrowed(&self.config.task, None);
            tracing::info!("a borrowed window went to the person, who took it over");
        }
    }

    /// Gives the front back to the person's app (unless they moved on) and
    /// parks the window again if it was parked.
    pub(super) fn restore_borrowed(&mut self) {
        if self.borrowed.is_some() && self.store.paused(&self.config.task) {
            self.hand_over_borrowed();
            return;
        }
        let Some(borrowed) = self.borrowed.take() else {
            return;
        };
        self.store.set_borrowed(&self.config.task, None);
        if let Some(pid) = super::front::give_back_to(
            borrowed.before,
            super::front::frontmost(),
            borrowed.window.pid,
        ) {
            super::front::give_back(pid);
        }
        if borrowed.parked {
            self.arrived.push(borrowed.window);
            self.place_arrivals();
        }
        tracing::info!("a borrowed front went back to the person");
    }

    /// Notes an acting call's outcome in its windows: failures count toward
    /// offering a brief focus.
    pub(super) fn count_failure(&mut self, resolved: &[Resolved], result: &Value) {
        let failed = result.get("isError").and_then(Value::as_bool) == Some(true)
            || super::super::child::envelope(result)
                .is_some_and(|envelope| envelope.get("ok") == Some(&Value::Bool(false)));
        for window in resolved {
            let Some(id) = &window.id else {
                continue;
            };
            if !failed {
                self.failures.remove(id);
                continue;
            }
            let count = self.failures.entry(id.clone()).or_insert(0);
            *count += 1;
            if *count == FAILS_BEFORE_FOCUS {
                // Measured: it doesn't take input off the screen. It waits
                // behind the person's windows instead, before any focus.
                let displays = self.screen.displays();
                let parked = self
                    .list(super::Look::Check)
                    .ok()
                    .and_then(|windows| windows.into_iter().find(|listed| &listed.id == id))
                    .filter(|listed| {
                        listed
                            .bounds
                            .is_some_and(|bounds| hangs_off(bounds, &displays))
                    });
                if let Some(parked) = parked {
                    self.put_behind(&parked, "it ignored actions off the screen");
                    continue;
                }
                self.notes.push(format!(
                    "{} ignored this in the background twice. If a fresh snapshot shows nothing \
                     changed, desktop_focus_window brings {id} forward briefly (Agent-Desk gives the \
                     person's window back right after).",
                    window.app
                ));
            }
        }
    }
}

/// Why a window comes forward: its app ignored background actions
/// (`failed`), or keys must reach it.
fn reason(failed: bool) -> &'static str {
    if failed {
        crate::assistant::guard::BORROW_IGNORED
    } else {
        crate::assistant::guard::BORROW_KEYS
    }
}

/// Whether a window of `bounds` hangs off every display but a sliver
/// (`space::off_screen`).
fn hangs_off(
    bounds: (f64, f64, f64, f64),
    displays: &[crate::assistant::computer::Display],
) -> bool {
    let displays: Vec<crate::assistant::space::Display> = displays
        .iter()
        .map(crate::assistant::space::Display::from)
        .collect();
    crate::assistant::space::off_screen(bounds, &displays)
}

/// Where a parked window comes on the screen while it's borrowed: inside
/// the person's display (else the main one).
fn on_screen_spot(displays: &[crate::assistant::computer::Display]) -> Option<(f64, f64)> {
    let display = displays
        .iter()
        .find(|display| display.has_focus)
        .or_else(|| displays.iter().find(|display| display.is_primary))
        .or(displays.first())?;
    Some((display.frame.0 + 80.0, display.frame.1 + 80.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parked_window_hangs_off_and_one_on_screen_does_not() {
        let display = crate::assistant::computer::Display {
            id: "1".into(),
            name: "Built-in".into(),
            frame: (0.0, 0.0, 1800.0, 1169.0),
            visible_frame: (0.0, 25.0, 1800.0, 1144.0),
            scale: 2.0,
            is_primary: true,
            has_focus: true,
        };
        assert!(hangs_off(
            (1799.0, 1168.0, 900.0, 600.0),
            &[display.clone()]
        ));
        assert!(!hangs_off((100.0, 100.0, 900.0, 600.0), &[display.clone()]));
        assert_eq!(on_screen_spot(&[display]), Some((80.0, 80.0)));
    }
}
