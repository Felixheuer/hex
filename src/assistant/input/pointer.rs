//! Where the agent last acted in a window, so the person sees it in the
//! live picture (the mini at the orb, the card shown large): the guard,
//! in the task's own process, notes the spot as a fraction of the window
//! (`<support>/assistant/pointer/<window number>.json`), and the overlay's
//! capture thread draws a ring there into each picture for a moment.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How long a ring shows after the action.
pub const SHOWS_MS: u64 = 2500;

/// One noted spot.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Mark {
    /// The spot as a fraction of the window (0..1 from its top-left).
    pub x: f64,
    pub y: f64,
    /// When, in ms since the epoch.
    pub at_ms: u64,
}

fn dir() -> Option<PathBuf> {
    crate::assistant::store::assistant_dir()
        .ok()
        .map(|dir| dir.join("pointer"))
}

fn file(dir: &Path, number: u32) -> PathBuf {
    dir.join(format!("{number}.json"))
}

/// Notes that the agent acted at `(x, y)` (fractions) in window `number`.
pub fn note(number: u32, (x, y): (f64, f64)) {
    if cfg!(test) {
        return;
    }
    let Some(dir) = dir() else {
        return;
    };
    let mark = Mark {
        x,
        y,
        at_ms: crate::desktop::events::now_ms(),
    };
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(json) = serde_json::to_vec(&mark)
        && let Err(error) = std::fs::write(file(&dir, number), json)
    {
        tracing::debug!(%error, "couldn't note where the agent acted");
    }
}

/// The spot to show in window `number` now, if one is recent.
pub fn recent(number: u32) -> Option<Mark> {
    let mark: Mark = serde_json::from_slice(&std::fs::read(file(&dir()?, number)).ok()?).ok()?;
    let now = crate::desktop::events::now_ms();
    (now.saturating_sub(mark.at_ms) < SHOWS_MS).then_some(mark)
}

/// Draws `mark` as a ring into an RGBA picture `width`×`height`, fading
/// with its age (`age_ms`).
pub fn stamp(rgba: &mut [u8], width: u32, height: u32, mark: &Mark, age_ms: u64) {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 || age_ms >= SHOWS_MS {
        return;
    }
    let alpha = 1.0 - age_ms as f64 / SHOWS_MS as f64;
    let cx = mark.x.clamp(0.0, 1.0) * w as f64;
    let cy = mark.y.clamp(0.0, 1.0) * h as f64;
    let radius = (w.max(h) as f64 / 45.0).clamp(8.0, 40.0);
    let thickness = (radius / 4.0).max(2.0);
    // The task colour family's warm accent, with a white rim inside.
    let ring = [255.0, 122.0, 41.0];
    let rim = [255.0, 255.0, 255.0];
    let reach = (radius + thickness + 1.0).ceil() as i64;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let px = cx as i64 + dx;
            let py = cy as i64 + dy;
            if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                continue;
            }
            let distance = ((px as f64 + 0.5 - cx).powi(2) + (py as f64 + 0.5 - cy).powi(2)).sqrt();
            let from_ring = (distance - radius).abs();
            let colour = if from_ring <= thickness / 2.0 {
                ring
            } else if (distance - (radius - thickness)).abs() <= 1.0 {
                rim
            } else if distance <= 2.5 {
                ring
            } else {
                continue;
            };
            let at = (py as usize * w + px as usize) * 4;
            for channel in 0..3 {
                let old = f64::from(rgba[at + channel]);
                rgba[at + channel] = (old + (colour[channel] - old) * alpha).round() as u8;
            }
        }
    }
}

/// [`stamp`]s the recent mark of window `number`, if any, into a picture.
pub fn stamp_recent(number: u32, rgba: &mut [u8], width: u32, height: u32) {
    if let Some(mark) = recent(number) {
        let age = crate::desktop::events::now_ms().saturating_sub(mark.at_ms);
        stamp(rgba, width, height, &mark, age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_is_drawn_around_the_spot_and_fades() {
        let (w, h) = (200_u32, 100_u32);
        let mut picture = vec![0_u8; (w * h * 4) as usize];
        let mark = Mark {
            x: 0.5,
            y: 0.5,
            at_ms: 0,
        };
        stamp(&mut picture, w, h, &mark, 0);
        let pixel = |x: usize, y: usize| picture[(y * w as usize + x) * 4];
        // The centre dot and the ring at radius 8 are coloured; far away
        // stays black.
        assert_eq!(pixel(100, 50), 255);
        assert_eq!(pixel(108, 50), 255);
        assert_eq!(pixel(5, 5), 0);
        // An old mark draws nothing.
        let mut blank = vec![0_u8; (w * h * 4) as usize];
        stamp(&mut blank, w, h, &mark, SHOWS_MS);
        assert!(blank.iter().all(|byte| *byte == 0));
        // A picture smaller than it says is left alone.
        let mut short = vec![0_u8; 10];
        stamp(&mut short, w, h, &mark, 0);
    }
}
