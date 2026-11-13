//! The enlarged live browser's panel: made with the other overlays, shown
//! on the display under the pointer when a preview in the orb's list is
//! pressed, keyed while it shows (Escape, and the person's typing once they
//! take over), and hidden, giving the keyboard back, when it closes.

use rune::{
    Align, DisplayTarget, KeyPolicy, OverlayHandle, OverlayInput, OverlayPlacement, OverlaySpec,
    ScreenEdge,
};

use super::super::app_live;
use super::super::board::Board;
use super::super::enlarged::{self, Enlarged};
use super::super::live_card::Actions;
use super::super::pictures;
use super::{FADE, OVERLAYS, leave_out_of_mission_control, options};

/// How far below the top of the display the card hangs.
const MARGIN: f32 = 64.0;

/// Whether the card shows (the wall under it then keeps itself open).
static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the card shows now.
pub(super) fn shown() -> bool {
    SHOWN.load(std::sync::atomic::Ordering::Acquire)
}

/// Adds the panel to the app.
pub(super) fn install<S: 'static>(app: &mut rune::App<S>) -> OverlayHandle<Enlarged> {
    let spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Agent-Desk live browser", enlarged::SIZE, enlarged::view)
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
    .on_created(|created| leave_out_of_mission_control(created.window_number))
    .release_gpu_when_hidden();
    app.overlay(
        spec,
        Enlarged::new(Actions {
            close,
            take_over: |state, id| {
                crate::assistant::service::take_over(id);
                if let Some(number) = shown_window(state) {
                    app_live::show(id, number);
                }
            },
            hand_back: |state, id| {
                if let Some(number) = shown_window(state) {
                    app_live::park(id, number);
                }
                crate::assistant::service::hand_back(id);
            },
            what_it_did: Some(|state, task, _| {
                // Its steps on the stage; the window stays closed.
                let task = task.to_owned();
                if let Some(overlays) = OVERLAYS.get() {
                    overlays.orb.update_if(move |orb, _| {
                        super::stage::show_steps(&orb.board, &task);
                        false
                    });
                }
                close(state);
            }),
        }),
    )
}

/// A preview pressed in the orb's list (whose board is `board`): opens it
/// large in the panel and takes the keyboard.
pub(super) fn open(board: &Board, task: &str, session: &str) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    pictures::view(Some((task, session)));
    // As large as the display under the pointer allows.
    let room = super::follow::pointer()
        .and_then(|at| {
            let all = super::follow::screens();
            super::super::screens::at(&all, at).map(|screen| screen.visible)
        })
        .map_or(enlarged::ROOM, |[_, _, w, h]| {
            enlarged::room_on((w as f32, h as f32), MARGIN)
        });
    overlays.enlarged.resize(enlarged::size_for(room));
    let (board, task, session) = (board.clone(), task.to_owned(), session.to_owned());
    overlays.enlarged.update(move |state, _| {
        state.room = room;
        enlarged::open(state, &task, &session, &board);
    });
    SHOWN.store(true, std::sync::atomic::Ordering::Release);
    overlays.enlarged.fade_in(FADE);
    overlays.enlarged.request_keys();
}

/// × , Escape, or What it did: closes the card, stops its large stream,
/// and hides the panel (which gives the keyboard back).
fn close(state: &mut Enlarged) {
    if enlarged::close(state) {
        pictures::view(None);
    }
    if let Some(overlays) = OVERLAYS.get() {
        overlays.enlarged.fade_out(FADE);
    }
    if SHOWN.swap(false, std::sync::atomic::Ordering::AcqRel) {
        super::wall::refocus();
    }
}

/// The app window the card shows, if it shows one.
fn shown_window(state: &Enlarged) -> Option<u32> {
    state
        .target
        .as_ref()
        .and_then(|(_, session)| app_live::number_of(session))
}

/// The orb's board changed: the card follows who has its task.
pub(super) fn follow(board: &Board) {
    let Some(overlays) = OVERLAYS.get() else {
        return;
    };
    let board = board.clone();
    overlays
        .enlarged
        .update_if(move |state, _| enlarged::follow(state, &board));
}

/// A frame or a page change: redraw the card while it shows.
pub(super) fn redraw() {
    if let Some(overlays) = OVERLAYS.get() {
        overlays
            .enlarged
            .update_if(|state, _| state.target.is_some());
    }
}
