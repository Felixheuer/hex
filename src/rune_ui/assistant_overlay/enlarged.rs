//! A task's browser, live and large, right in the overlay: pressing a
//! preview in the orb's list opens it here, on the display the pointer is
//! on, with Take over / Hand back below it, What it did and × above it,
//! and Escape to close. The panel takes the keyboard only while it shows
//! (so Escape and, once taken over, typing reach it) and gives it up when
//! it hides.
//!
//! The rules are plain functions on [`Enlarged`] ([`open`], [`close`],
//! [`follow`]); the host adds the stream ([`live::view`]) and the panel's
//! showing and focus around them.

use rune::{Cx, El, col};

use super::super::controls::Paint;
use super::board::{Board, Phase};
use super::live_card::{self, Driver, PAD};

/// The largest picture the panel shows, in points.
pub(crate) const ROOM: (f32, f32) = (720.0, 450.0);
/// The panel: the card at its largest, and room for its shadow.
pub(crate) const SIZE: (f32, f32) = size_for(ROOM);

/// The panel around a picture of at most `room`.
pub(crate) const fn size_for(room: (f32, f32)) -> (f32, f32) {
    (room.0 + PAD * 2.0 + 40.0, room.1 + 170.0)
}

/// The picture's room on a display whose visible area is `visible`, the
/// card hanging `margin` below its top: as large as the display allows
/// ("full screen" from a tile), never smaller than [`ROOM`].
pub(crate) fn room_on(visible: (f32, f32), margin: f32) -> (f32, f32) {
    let (w, h) = SIZE;
    (
        (visible.0 - 80.0 - (w - ROOM.0)).max(ROOM.0),
        (visible.1 - margin - 24.0 - (h - ROOM.1)).max(ROOM.1),
    )
}

/// The enlarged card's state.
pub(crate) struct Enlarged {
    /// The browser it shows: (task, session); `None` while closed.
    pub(crate) target: Option<(String, String)>,
    /// Who has the task, as the orb's board last said.
    pub(crate) driver: Driver,
    /// The picture's room ([`room_on`]).
    pub(crate) room: (f32, f32),
    pub(crate) actions: live_card::Actions<Enlarged>,
}

impl Enlarged {
    pub(crate) fn new(actions: live_card::Actions<Enlarged>) -> Self {
        Self {
            target: None,
            driver: Driver::Agent,
            room: ROOM,
            actions,
        }
    }
}

/// Opens `session` of `task` (a preview pressed in the list); whether it
/// changed what shows.
pub(crate) fn open(state: &mut Enlarged, task: &str, session: &str, board: &Board) -> bool {
    let target = Some((task.to_owned(), session.to_owned()));
    let changed = state.target != target;
    state.target = target;
    follow(state, board);
    changed
}

/// Closes the card; whether it was open.
pub(crate) fn close(state: &mut Enlarged) -> bool {
    state.driver = Driver::Agent;
    state.target.take().is_some()
}

/// Who has the task now, from `board`; whether that changed.
pub(crate) fn follow(state: &mut Enlarged, board: &Board) -> bool {
    let Some((task, _)) = &state.target else {
        return false;
    };
    let phase = board.tasks.iter().find(|t| &t.id == task).map(|t| t.phase);
    let driver = driver_for(phase);
    let changed = state.driver != driver;
    state.driver = driver;
    changed
}

/// Who has a task in `phase` (`None`: it's gone from the board).
pub(crate) fn driver_for(phase: Option<Phase>) -> Driver {
    match phase {
        Some(Phase::Paused) => Driver::Person,
        Some(phase) if phase.live() => Driver::Agent,
        _ => Driver::Finished,
    }
}

pub(crate) fn view(state: &Enlarged, cx: &mut Cx<Enlarged>) -> El {
    super::wheel::begin(super::wheel::Surface::Enlarged);
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let root = col()
        .key("enlarged-root")
        .w_full()
        .h_full()
        .padding(20.0)
        .items_center()
        .justify_start();
    let Some((task, session)) = &state.target else {
        return root;
    };
    let close = state.actions.close;
    root.child(
        live_card::card(p, task, session, state.driver, state.room, state.actions)
            .on_key(move |s: &mut Enlarged, key: &rune::KeyEvent, _| {
                if key.key == rune::Key::Escape {
                    close(s);
                    true
                } else {
                    false
                }
            })
            .transition(rune::Transition::scale(0.96).combined(rune::Transition::opacity())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::{Event, Origin};
    use std::time::Instant;

    fn actions() -> live_card::Actions<Enlarged> {
        live_card::Actions {
            close: |s| {
                close(s);
            },
            take_over: |_, _| {},
            hand_back: |_, _| {},
            what_it_did: None,
        }
    }

    fn board_with(task: &str) -> Board {
        let mut board = Board::default();
        let now = Instant::now();
        board.apply(
            &Event::Started {
                conversation: task.into(),
                origin: Origin::Background,
                question: "Configure the Mac mini".into(),
            },
            None,
            now,
        );
        board
    }

    #[test]
    fn a_preview_opens_large_is_taken_over_handed_back_and_closed() {
        let mut board = board_with("t");
        let mut state = Enlarged::new(actions());
        // Thumbnail → enlarged.
        assert!(open(&mut state, "t", "hex-t", &board));
        assert_eq!(state.target, Some(("t".into(), "hex-t".into())));
        assert_eq!(state.driver, Driver::Agent);
        assert!(!open(&mut state, "t", "hex-t", &board), "already open");
        // Taken over: the board says Paused.
        board.apply(
            &Event::Paused {
                conversation: "t".into(),
            },
            None,
            Instant::now(),
        );
        assert!(follow(&mut state, &board));
        assert_eq!(state.driver, Driver::Person);
        // Handed back.
        board.apply(
            &Event::Resumed {
                conversation: "t".into(),
            },
            None,
            Instant::now(),
        );
        assert!(follow(&mut state, &board));
        assert_eq!(state.driver, Driver::Agent);
        // Closed.
        assert!(close(&mut state));
        assert_eq!(state.target, None);
        assert!(!close(&mut state));
        assert!(!follow(&mut state, &board), "nothing to follow once closed");
    }

    #[test]
    fn a_task_gone_from_the_board_is_finished() {
        assert_eq!(driver_for(None), Driver::Finished);
        assert_eq!(driver_for(Some(Phase::Done)), Driver::Finished);
        assert_eq!(driver_for(Some(Phase::Running)), Driver::Agent);
        assert_eq!(driver_for(Some(Phase::Paused)), Driver::Person);
    }

    #[test]
    fn a_tile_opens_the_card_as_large_as_the_display_allows() {
        // This Mac's 1800 × 1132 below the menu bar.
        let room = room_on((1800.0, 1132.0), 64.0);
        assert!(room.0 > 1500.0 && room.1 > 800.0, "{room:?}");
        let size = size_for(room);
        assert!(size.0 <= 1800.0 && size.1 + 64.0 <= 1132.0, "{size:?}");
        // A small display keeps the card's own size.
        assert_eq!(room_on((760.0, 560.0), 64.0), ROOM);
    }

    #[test]
    fn the_panel_holds_the_card_at_its_largest() {
        let (w, h) = live_card::fitted((1280.0, 800.0), ROOM);
        assert!(w + PAD * 2.0 <= SIZE.0);
        assert!(h + 150.0 <= SIZE.1);
    }
}
