//! The live wall's panel: made with the other overlays, shown on the
//! display under the pointer when "All N live" on a job's row is pressed,
//! keyed while it shows (Escape), and hidden when it closes: ×, Escape, or
//! a click outside it (the panel loses the keyboard), but not while the
//! enlarged card a tile opened is on top. Its jobs' browsers and windows
//! stream only while it shows (`live::wall`, `app_live::wall`).

use std::sync::atomic::{AtomicBool, Ordering};

use rune::{
    Align, DisplayTarget, KeyPolicy, OverlayHandle, OverlayInput, OverlayPlacement, OverlaySpec,
    ScreenEdge,
};

use super::super::app_live;
use super::super::board::Board;
use super::super::live;
use super::super::wall::{self, Actions, Wall};
use super::{FADE, OVERLAYS, leave_out_of_mission_control, options};

/// How far below the top of the display the wall hangs.
const MARGIN: f32 = 40.0;

/// Whether the wall shows, for the enlarged card to hand the keyboard back.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Adds the panel to the app.
pub(super) fn install<S: 'static>(app: &mut rune::App<S>) -> OverlayHandle<Wall> {
    let spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Hex live wall", wall::SIZE, wall::view)
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
        if key.key == rune::Key::Escape && state.lead.is_some() {
            close(state);
            true
        } else {
            false
        }
    })
    .on_focus(|state, focused, _| {
        // A click outside: it goes. A tile's enlarged card taking the
        // keyboard leaves it be.
        if !focused && state.lead.is_some() && !super::enlarged::shown() {
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
        }),
    )
}

/// "All N live" on `lead`'s card (whose board is `board`): shows every
/// browser and window of it and its helpers, streaming from now on.
pub(super) fn open(board: &Board, lead: &str) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    stream(&wall::tasks(board, lead));
    let (board, lead) = (board.clone(), lead.to_owned());
    overlays.wall.update(move |state, _| {
        state.lead = Some(lead);
        state.board = board;
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

/// × or Escape: stops the wall's streams and hides it.
fn close(state: &mut Wall) {
    state.lead = None;
    SHOWN.store(false, Ordering::Release);
    stream(&[]);
    if let Some(overlays) = OVERLAYS.get() {
        overlays.wall.fade_out(FADE);
    }
}

fn stream(tasks: &[String]) {
    live::wall(tasks);
    app_live::wall(tasks);
}

/// The orb's board changed: the wall follows its team (a helper started
/// joins it at once).
pub(super) fn follow(board: &Board) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let board = board.clone();
    overlays.wall.update_if(move |state, _| {
        let Some(lead) = state.lead.clone() else {
            return false;
        };
        stream(&wall::tasks(&board, &lead));
        state.board = board;
        true
    });
}

/// A frame or a page change: redraw the wall while it shows.
pub(super) fn redraw() {
    if let Some(overlays) = OVERLAYS.get() {
        overlays.wall.update_if(|state, _| state.lead.is_some());
    }
}
