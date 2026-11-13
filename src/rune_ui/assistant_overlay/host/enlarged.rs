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
use super::super::live::{self, Viewer};
use super::super::live_card::Actions;
use super::{FADE, OVERLAYS, leave_out_of_mission_control, open_chats, options};

/// How far below the top of the display the card hangs.
const MARGIN: f32 = 64.0;

/// Adds the panel to the app.
pub(super) fn install<S: 'static>(app: &mut rune::App<S>) -> OverlayHandle<Enlarged> {
    let spec = OverlaySpec {
        interactive: true,
        ..OverlaySpec::new("Hex live browser", enlarged::SIZE, enlarged::view)
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
            open_chats: Some(|state, task, session| {
                live::request_view(task, session);
                open_chats(task);
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
    view(task, session);
    let (board, task, session) = (board.clone(), task.to_owned(), session.to_owned());
    overlays.enlarged.update(move |state, _| {
        enlarged::open(state, &task, &session, &board);
    });
    overlays.enlarged.fade_in(FADE);
    overlays.enlarged.request_keys();
}

/// × , Escape, or Open in Chats: closes the card, stops its large stream,
/// and hides the panel (which gives the keyboard back).
fn close(state: &mut Enlarged) {
    if enlarged::close(state) {
        live::view(Viewer::Overlay, None);
        app_live::view(None);
    }
    if let Some(overlays) = OVERLAYS.get() {
        overlays.enlarged.fade_out(FADE);
    }
}

/// Streams what the card shows: a browser from `live`, an app window from
/// `app_live`.
fn view(task: &str, session: &str) {
    match app_live::number_of(session) {
        Some(number) => {
            live::view(Viewer::Overlay, None);
            app_live::view(Some((task, number)));
        }
        None => {
            app_live::view(None);
            live::view(Viewer::Overlay, Some((task, session)));
        }
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
