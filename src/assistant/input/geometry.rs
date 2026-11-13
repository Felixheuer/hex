//! Coordinates for background input. A window shot is in pixels (a Retina
//! display has two per point); macOS events are in global points (the main
//! display's top-left at 0,0, y down); AppKit's window-local location is in
//! points from the window's top-left. The agent picks a spot in the shot;
//! [`Frame::target`] turns it into both.

/// A window's frame in global points, and its display's backing scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Pixels per point of the display it is on (2 on Retina).
    pub scale: f64,
}

/// Where an event lands: on the screen (global points) and in the window
/// (points from its top-left corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub screen: (f64, f64),
    pub local: (f64, f64),
}

/// How a spot is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    /// Pixels of a shot `width`×`height` of the window (`hex_window_shot`).
    ShotPixels { width: u32, height: u32 },
    /// Points from the window's top-left (what `hex_read_window` gives).
    Points,
}

impl Frame {
    /// The spot `(x, y)` in `units` as a target; `None` outside the window.
    pub fn target(&self, (x, y): (f64, f64), units: Units) -> Option<Target> {
        if !(x.is_finite() && y.is_finite()) || self.width <= 0.0 || self.height <= 0.0 {
            return None;
        }
        let (local_x, local_y) = match units {
            Units::Points => (x, y),
            Units::ShotPixels { width, height } => {
                if width == 0 || height == 0 {
                    return None;
                }
                // The shot may be scaled down (a long edge cap): map by its
                // own size, not by the display's scale.
                (
                    x * self.width / f64::from(width),
                    y * self.height / f64::from(height),
                )
            }
        };
        if local_x < 0.0 || local_y < 0.0 || local_x >= self.width || local_y >= self.height {
            return None;
        }
        Some(Target {
            screen: (self.x + local_x, self.y + local_y),
            local: (local_x, local_y),
        })
    }

    /// The shot's size in pixels at the display's own resolution.
    pub fn native_pixels(&self) -> (u32, u32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (self.width * scale).round().max(1.0) as u32,
            (self.height * scale).round().max(1.0) as u32,
        )
    }

    /// The window's centre point of an element frame given in global
    /// points (an Accessibility frame), as a target; `None` when it isn't
    /// inside this window.
    pub fn element_centre(&self, (x, y, width, height): (f64, f64, f64, f64)) -> Option<Target> {
        let centre = (x + width / 2.0 - self.x, y + height / 2.0 - self.y);
        self.target(centre, Units::Points)
    }

    /// Where a click goes to eat AppKit's first-click activation: just
    /// outside the window's top-left corner, so nothing in it is hit.
    pub fn primer(&self) -> Target {
        Target {
            screen: (self.x - 1.0, self.y - 1.0),
            local: (-1.0, -1.0),
        }
    }

    /// Where the spot sits as a fraction of the window (0..1 each way), for
    /// the ring in the live picture.
    pub fn fraction(&self, target: &Target) -> (f64, f64) {
        (
            (target.local.0 / self.width).clamp(0.0, 1.0),
            (target.local.1 / self.height).clamp(0.0, 1.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RETINA: Frame = Frame {
        x: 1360.0,
        y: 869.0,
        width: 420.0,
        height: 260.0,
        scale: 2.0,
    };

    #[test]
    fn shot_pixels_on_a_retina_display_are_half_points() {
        let shot = Units::ShotPixels {
            width: 840,
            height: 520,
        };
        let target = RETINA.target((100.0, 50.0), shot).unwrap();
        assert_eq!(target.local, (50.0, 25.0));
        assert_eq!(target.screen, (1410.0, 894.0));
        assert_eq!(RETINA.native_pixels(), (840, 520));
    }

    #[test]
    fn a_scaled_down_shot_maps_by_its_own_size() {
        // The long edge capped at 420 px: one pixel is one point here.
        let shot = Units::ShotPixels {
            width: 420,
            height: 260,
        };
        let target = RETINA.target((210.0, 130.0), shot).unwrap();
        assert_eq!(target.local, (210.0, 130.0));
        assert_eq!(RETINA.fraction(&target), (0.5, 0.5));
    }

    #[test]
    fn points_are_window_local_and_the_origin_may_be_negative() {
        // A window on a display left of the main one, or parked: negative
        // global coordinates are fine.
        let left = Frame {
            x: -1200.0,
            y: -40.0,
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        };
        let target = left.target((10.0, 20.0), Units::Points).unwrap();
        assert_eq!(target.screen, (-1190.0, -20.0));
        assert_eq!(target.local, (10.0, 20.0));
    }

    #[test]
    fn spots_outside_the_window_are_refused() {
        let shot = Units::ShotPixels {
            width: 840,
            height: 520,
        };
        assert!(RETINA.target((840.0, 10.0), shot).is_none());
        assert!(RETINA.target((-1.0, 10.0), shot).is_none());
        assert!(RETINA.target((f64::NAN, 10.0), shot).is_none());
        let empty = Units::ShotPixels {
            width: 0,
            height: 0,
        };
        assert!(RETINA.target((1.0, 1.0), empty).is_none());
    }

    #[test]
    fn an_elements_centre_and_the_primer() {
        let target = RETINA
            .element_centre((1366.0, 901.0, 100.0, 20.0))
            .unwrap();
        assert_eq!(target.local, (56.0, 42.0));
        assert!(RETINA.element_centre((0.0, 0.0, 10.0, 10.0)).is_none());
        let primer = RETINA.primer();
        assert_eq!(primer.local, (-1.0, -1.0));
        assert_eq!(primer.screen, (1359.0, 868.0));
    }
}
