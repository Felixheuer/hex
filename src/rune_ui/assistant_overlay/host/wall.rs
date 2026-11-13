//! The live wall's panel: made with the other overlays, shown on the
//! display under the pointer when "All N live" on a job's row, or "All
//! live" beside the orb, is pressed, keyed while it shows (Escape), and
//! hidden when it closes: ×, Escape, a click beside the panel, or a click
//! outside the window (the panel loses the keyboard), but not while the
//! enlarged card a tile opened is on top. Its tasks' browsers and windows
//! stream only while it shows (`live::wall`, `app_live::wall`).

use std::sync::atomic::{AtomicBool, Ordering};

use rune::{
    Align, DisplayTarget, KeyPolicy, OverlayHandle, OverlayInput, OverlayPlacement, OverlaySpec,
    ScreenEdge,
};

use super::super::board::Board;
use super::super::minis;
use super::super::wall::{self, Actions, Scope, Wall};
use super::{FADE, OVERLAYS, leave_out_of_mission_control, options};

/// How far below the top of the display the wall hangs.
const MARGIN: f32 = 40.0;

/// Whether the wall shows, for the enlarged card to hand the keyboard back.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Adds the panel to the app.
pub(super) fn install<S: 'static>(app: &mut rune::App<S>) -> OverlayHandle<Wall> {
    let spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Agent-Desk live wall", wall::SIZE, wall::view)
    }
    .options(rune::OverlayOptions {
        keys: KeyPolicy::OnDemand,
        display: DisplayTarget::UnderPointer,
        placement: Some(OverlayPlacement::Edge {
            edge: ScreenEdge::Top,
            margin: MARGIN,
            align: Align::Center,
            inset: 0.0,
        }),
        ..options(OverlayInput::Regions)
    })
    // Escape wherever the focus is inside the panel (or none is).
    .on_key(|state, key, _| {
        if key.key == rune::Key::Escape && state.scope.is_some() {
            close(state);
            true
        } else {
            false
        }
    })
    .on_focus(|state, focused, _| {
        // A click outside: it goes. A tile's enlarged card taking the
        // keyboard leaves it be.
        if !focused && state.scope.is_some() && !super::enlarged::shown() {
            close(state);
        }
    })
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    app.overlay(
        spec,
        Wall::new(Actions {
            close,
            open: |state, task, session| {
                super::enlarged::open(&state.board, task, session);
            },
            seen: |state, task, session, on| {
                let picture = (task.to_owned(), session.to_owned());
                if on {
                    state.seen.insert(picture);
                } else {
                    state.seen.remove(&picture);
                }
                let seen: Vec<(String, String)> = state.seen.iter().cloned().collect();
                super::super::live::wall_seen(Some(&seen));
            },
        }),
    )
}

/// The live overview's "Large": the same pictures on the wall.
pub(super) fn open_scope(board: &Board, scope: Scope) {
    show(board, scope);
}

fn show(board: &Board, scope: Scope) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let desk = minis::now();
    stream(&wall::tasks(board, &desk, &scope));
    // As large as it is, but never past the display under the pointer.
    let size = super::follow::pointer()
        .and_then(|at| {
            let all = super::follow::screens();
            super::super::screens::at(&all, at).map(|screen| screen.visible)
        })
        .map_or(wall::SIZE, |[_, _, w, h]| {
            wall::fitted((w as f32, h as f32), MARGIN)
        });
    overlays.wall.resize(size);
    let board = board.clone();
    overlays.wall.update(move |state, _| {
        state.scope = Some(scope);
        state.board = board;
        state.desk = desk;
        state.size = size;
        state.seen.clear();
    });
    SHOWN.store(true, Ordering::Release);
    overlays.wall.fade_in(FADE);
    overlays.wall.request_keys();
}

/// The enlarged card above the wall closed: the wall takes the keyboard
/// back (for Escape, and to close on a click outside).
pub(super) fn refocus() {
    if SHOWN.load(Ordering::Acquire)
        && let Some(overlays) = OVERLAYS.get()
    {
        overlays.wall.request_keys();
    }
}

/// ×, Escape, or a click outside: stops the wall's streams and hides it.
fn close(state: &mut Wall) {
    state.scope = None;
    SHOWN.store(false, Ordering::Release);
    state.seen.clear();
    super::super::live::wall_seen(None);
    stream(&[]);
    if let Some(overlays) = OVERLAYS.get() {
        overlays.wall.fade_out(FADE);
    }
}

fn stream(tasks: &[String]) {
    super::super::pictures::wall(tasks);
}

/// The orb's board (or the desk) changed: the wall follows its tasks (a
/// helper started joins it at once).
pub(super) fn follow(board: &Board) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let board = board.clone();
    let desk = minis::now();
    overlays.wall.update_if(move |state, _| {
        let Some(scope) = state.scope.clone() else {
            return false;
        };
        stream(&wall::tasks(&board, &desk, &scope));
        state.board = board;
        state.desk = desk;
        true
    });
}

/// A frame or a page change: redraw the wall while it shows.
pub(super) fn redraw() {
    if let Some(overlays) = OVERLAYS.get() {
        overlays.wall.update_if(|state, _| state.scope.is_some());
    }
}
