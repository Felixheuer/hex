//! One of a task's browsers, live and large, over Chats: opened with "Open
//! in Chats" from the overlay's enlarged card. The card is the overlay's
//! (`assistant_overlay::live_card`): Take over hands the task to the person
//! (its browser and desktop actions wait) and lets them click, scroll, and
//! type in the page; Hand back gives it to the agent again, which looks
//! afresh before it acts. Both share the browser's one stream.

use rune::{El, el};

use super::super::assistant_overlay::live::{self, Viewer};
use super::super::assistant_overlay::live_card::{self, Actions, Driver};
use super::super::controls::Paint;
use super::{Hold, Ui};

/// The picture's largest size, in points.
const ROOM: (f32, f32) = (880.0, 560.0);

/// Opens `session` of `task` large.
pub(super) fn open(ui: &mut Ui, task: String, session: String) {
    live::view(Viewer::Chats, Some((&task, &session)));
    ui.chats.live = Some((task, session));
}

/// Closes the large view; the stream goes back to small, or stops.
pub(super) fn close(ui: &mut Ui) -> bool {
    live::view(Viewer::Chats, None);
    ui.chats.live.take().is_some()
}

/// The picture's size for a page of `viewport`, filling the room it has
/// and keeping the page's proportions.
#[cfg(test)]
fn fitted(viewport: (f32, f32)) -> (f32, f32) {
    live_card::fitted(viewport, ROOM)
}

/// The sheet, while a browser shows large.
pub(super) fn sheet(ui: &Ui, p: Paint) -> Option<El> {
    let (task, session) = ui.chats.live.clone()?;
    let k = p.k;
    let driver = match ui.chats.task(&task) {
        None => Driver::Finished,
        Some(status) if matches!(ui.chats.hold(status), Some(Hold::Paused)) => Driver::Person,
        Some(_) => Driver::Agent,
    };
    let card = live_card::card(
        p,
        &task,
        &session,
        driver,
        ROOM,
        Actions {
            close: |ui: &mut Ui| {
                close(ui);
            },
            take_over: |ui: &mut Ui, id| ui.chats.take_over(id),
            hand_back: |ui: &mut Ui, id| ui.chats.hand_back(id),
            what_it_did: None,
        },
    );
    Some(
        el().key("live-sheet")
            // A scrim, not a control: Close and Escape close it too.
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(k.c.bg_deep.alpha(0.6))
            .on_press(|ui: &mut Ui| {
                close(ui);
            })
            .child(card)
            .modal(true, |ui: &mut Ui| {
                close(ui);
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_keeps_the_pages_proportions() {
        assert_eq!(fitted((1280.0, 720.0)), (880.0, 495.0));
        assert_eq!(fitted((800.0, 1200.0)), (373.33334, 560.0));
        assert_eq!(fitted((0.0, 0.0)), (880.0, 550.0));
    }
}
