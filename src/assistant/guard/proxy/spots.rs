//! Where a task acts in its window, for the ring on its live picture (the
//! minis, the wall, the card shown large): the element's center as a
//! fraction of the window, noted as `hex_window_input` notes its spots
//! (`input::pointer`), so every action through Cua Driver shows where it
//! landed, not only those by a window's picture.
//!
//! Cua Driver's own agent cursor is an overlay on the screen: it shows only
//! where a window is on the screen, and its overlay needs Cua's own service
//! (a driver run inside Agent-Desk's process, as tasks run it, has none:
//! `facility_unavailable`). Parked windows' pictures get Agent-Desk's ring.

use serde_json::{Map, Value};

use super::super::child::Desktop;
use super::{Guard, Look, Resolved, text};

impl<D: Desktop> Guard<D> {
    /// Notes where `tool` acted, when its element's frame is known: the
    /// window's number and the spot.
    pub(super) fn spot(
        &mut self,
        args: &Map<String, Value>,
        resolved: &[Resolved],
        result: &Value,
    ) -> Option<(u32, (f64, f64))> {
        if self.reader || result["isError"] == true {
            return None;
        }
        let window = resolved.first()?.id.clone()?;
        let reference = text(args, "ref_id").or_else(|| {
            args.get("steps")
                .and_then(Value::as_array)?
                .iter()
                .rev()
                .find_map(|step| step.as_object().and_then(|step| text(step, "ref_id")))
        })?;
        let frame = self.learned.vision.lock().frame_of(&reference)?;
        let bounds = self
            .list(Look::Check)
            .ok()?
            .into_iter()
            .find(|on| on.id == window)?
            .bounds?;
        let spot = within(frame, bounds)?;
        let number = crate::assistant::computer::window_number(&window)?;
        crate::assistant::input::pointer::note(number, spot);
        self.spotted = Some((number, spot));
        Some((number, spot))
    }
}

/// An element frame's center as a fraction of its window's bounds (both
/// global points); `None` outside it.
pub fn within(frame: (f64, f64, f64, f64), window: (f64, f64, f64, f64)) -> Option<(f64, f64)> {
    let (x, y, width, height) = frame;
    let (left, top, across, down) = window;
    if across <= 0.0 || down <= 0.0 {
        return None;
    }
    let at = (
        (x + width / 2.0 - left) / across,
        (y + height / 2.0 - top) / down,
    );
    ((0.0..=1.0).contains(&at.0) && (0.0..=1.0).contains(&at.1)).then_some(at)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_spot_is_the_element_s_center_across_its_window() {
        let window = (100.0, 50.0, 400.0, 200.0);
        assert_eq!(
            super::within((300.0, 140.0, 0.0, 20.0), window),
            Some((0.5, 0.5))
        );
        assert_eq!(
            super::within((95.0, 45.0, 10.0, 10.0), window),
            Some((0.0, 0.0))
        );
        assert_eq!(super::within((900.0, 40.0, 10.0, 10.0), window), None);
        assert_eq!(
            super::within((1.0, 1.0, 1.0, 1.0), (0.0, 0.0, 0.0, 0.0)),
            None
        );
    }
}
