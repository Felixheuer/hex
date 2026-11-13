//! One window's picture by its CoreGraphics number, wherever it is: Hex's
//! live pictures of the app windows a job works in.

use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use super::display_of;

/// One picture of the window numbered `window_id` (a CoreGraphics window
/// number), wherever it is: covered, on another display, or hanging off
/// the screen (ScreenCaptureKit captures a window's own content; a
/// minimized window gives nothing). At the window's own pixels (its points
/// times its display's scale), its long side at most `max_long_edge`.
/// Returns (width, height, RGBA).
pub fn capture_window(window_id: u32, max_long_edge: u32) -> Option<(u32, u32, Vec<u8>)> {
    let content = SCShareableContent::get().ok()?;
    let window = content
        .windows()
        .into_iter()
        .find(|window| window.window_id() == window_id)?;
    let frame = window.frame();
    let screens = crate::screens::displays();
    let rect = [
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    ];
    let (_, scale) = display_of(&screens, rect);
    let (width, height) = window_pixels(
        (frame.size.width, frame.size.height),
        scale,
        max_long_edge,
    )?;
    let filter = SCContentFilter::create().with_window(&window).build().ok()?;
    let config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_shows_cursor(false);
    let image = SCScreenshotManager::capture_image(&filter, &config).ok()?;
    let (w, h) = (
        u32::try_from(image.width()).ok()?,
        u32::try_from(image.height()).ok()?,
    );
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = image.rgba_data().ok()?;
    (rgba.len() == (w as usize) * (h as usize) * 4).then_some((w, h, rgba))
}

/// A window of `points` on a display of `scale`, in pixels, its long side
/// at most `max_long_edge`; `None` for an empty window.
pub fn window_pixels(points: (f64, f64), scale: f64, max_long_edge: u32) -> Option<(u32, u32)> {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (w, h) = (points.0 * scale, points.1 * scale);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let long = w.max(h);
    let fit = (f64::from(max_long_edge) / long).min(1.0);
    Some((
        ((w * fit).round() as u32).max(1),
        ((h * fit).round() as u32).max(1),
    ))
}

#[cfg(test)]
mod capture_window_tests {
    #[test]
    fn a_window_is_captured_at_its_own_pixels_up_to_the_cap() {
        assert_eq!(super::window_pixels((800.0, 600.0), 2.0, 4096), Some((1600, 1200)));
        assert_eq!(super::window_pixels((2560.0, 1440.0), 2.0, 2560), Some((2560, 1440)));
        assert_eq!(super::window_pixels((100.0, 0.0), 2.0, 2560), None);
        assert_eq!(super::window_pixels((800.0, 600.0), 0.0, 4096), Some((800, 600)));
    }
}
