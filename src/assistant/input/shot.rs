//! A window's picture for the vision path (`hex_window_shot`): taken with
//! ScreenCaptureKit wherever the window is (parked off the screen, covered),
//! at the display's own pixels, written as a PNG the agent can look at.

use std::path::{Path, PathBuf};

/// The longest side a shot is taken at (a 5K window at 2× stays sharp).
pub const MAX_EDGE: u32 = 4096;

/// One shot.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub path: PathBuf,
    /// Its size in pixels.
    pub width: u32,
    pub height: u32,
    /// A cheap fingerprint of its pixels, to tell whether a later shot
    /// changed.
    pub fingerprint: u64,
    /// The share of a coarse grid that differs from the shot before, when
    /// there was one of the same size (0..1).
    pub changed: Option<f64>,
}

/// A fingerprint of a picture (every 97th byte and the length).
pub fn fingerprint(rgba: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ rgba.len() as u64;
    for byte in rgba.iter().step_by(97) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The share of 16×16 cells whose average differs by more than a little
/// between two pictures of the same size; `None` when sizes differ.
pub fn changed_share(a: &[u8], b: &[u8], width: u32, height: u32) -> Option<f64> {
    let (w, h) = (width as usize, height as usize);
    if a.len() != b.len() || a.len() < w * h * 4 || w == 0 || h == 0 {
        return None;
    }
    const CELLS: usize = 16;
    let mut differing = 0;
    for cy in 0..CELLS {
        for cx in 0..CELLS {
            let (x0, x1) = (cx * w / CELLS, ((cx + 1) * w / CELLS).max(cx * w / CELLS + 1));
            let (y0, y1) = (cy * h / CELLS, ((cy + 1) * h / CELLS).max(cy * h / CELLS + 1));
            let mut sum = 0_u64;
            let mut count = 0_u64;
            for y in (y0..y1.min(h)).step_by(2) {
                for x in (x0..x1.min(w)).step_by(2) {
                    let at = (y * w + x) * 4;
                    for channel in 0..3 {
                        sum += u64::from(a[at + channel].abs_diff(b[at + channel]));
                    }
                    count += 3;
                }
            }
            if count > 0 && sum / count > 6 {
                differing += 1;
            }
        }
    }
    Some(f64::from(differing) / (CELLS * CELLS) as f64)
}

/// `rgba` (`width`×`height`) scaled down so its long side is at most
/// `edge` (box filter); unchanged when it fits.
pub fn fit(rgba: &[u8], width: u32, height: u32, edge: u32) -> (u32, u32, Vec<u8>) {
    let long = width.max(height);
    if long <= edge || edge == 0 || rgba.len() < (width * height * 4) as usize {
        return (width, height, rgba.to_vec());
    }
    let ratio = f64::from(edge) / f64::from(long);
    let (to_w, to_h) = (
        ((f64::from(width) * ratio).round() as u32).max(1),
        ((f64::from(height) * ratio).round() as u32).max(1),
    );
    let mut out = vec![0_u8; (to_w * to_h * 4) as usize];
    for y in 0..to_h {
        let (y0, y1) = (y * height / to_h, ((y + 1) * height / to_h).max(y * height / to_h + 1));
        for x in 0..to_w {
            let (x0, x1) = (x * width / to_w, ((x + 1) * width / to_w).max(x * width / to_w + 1));
            let mut sum = [0_u32; 4];
            let mut count = 0;
            for sy in y0..y1.min(height) {
                for sx in x0..x1.min(width) {
                    let at = ((sy * width + sx) * 4) as usize;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(rgba[at + channel]);
                    }
                    count += 1;
                }
            }
            let at = ((y * to_w + x) * 4) as usize;
            for channel in 0..4 {
                out[at + channel] = (sum[channel] / count.max(1)) as u8;
            }
        }
    }
    (to_w, to_h, out)
}

/// RGBA `rgba` (`width`×`height`) as PNG bytes.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("png: {error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("png: {error}"))?;
    }
    Ok(bytes)
}

/// Writes RGBA `rgba` (`width`×`height`) as a PNG at `path`.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("png: {error}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| format!("png: {error}"))
}

/// Window `number`'s picture now, as RGBA.
pub fn capture(number: u32) -> Result<(u32, u32, Vec<u8>), String> {
    #[cfg(target_os = "macos")]
    {
        scrawl_capture::capture::capture_window(number, MAX_EDGE).ok_or_else(|| {
            "Hex couldn't take the window's picture (it closed or is minimized, or Screen \
             Recording isn't allowed)."
                .to_owned()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = number;
        Err("A window's picture works on macOS only.".into())
    }
}

/// Takes window `number`'s picture into `dir` (`window-<number>-<n>.png`),
/// comparing it with `before` (the last picture's pixels) when given.
pub fn take(number: u32, dir: &Path, before: Option<&[u8]>) -> Result<(Shot, Vec<u8>), String> {
    let (width, height, rgba) = capture(number)?;
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let stamp = crate::desktop::events::now_ms();
    let path = dir.join(format!("window-{number}-{stamp}.png"));
    write_png(&path, width, height, &rgba)?;
    let changed = before.and_then(|before| changed_share(before, &rgba, width, height));
    Ok((
        Shot {
            path,
            width,
            height,
            fingerprint: fingerprint(&rgba),
            changed,
        },
        rgba,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_told_by_its_share_of_the_picture() {
        let (w, h) = (64_u32, 64_u32);
        let a = vec![255_u8; (w * h * 4) as usize];
        let mut b = a.clone();
        assert_eq!(changed_share(&a, &b, w, h), Some(0.0));
        // The top-left quarter goes black.
        for y in 0..32 {
            for x in 0..32 {
                let at = ((y * w + x) * 4) as usize;
                b[at..at + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        assert_eq!(changed_share(&a, &b, w, h), Some(0.25));
        assert_ne!(fingerprint(&a), fingerprint(&b));
        assert_eq!(changed_share(&a, &b[..16], w, h), None);
    }

    #[test]
    fn a_shot_is_written_as_a_png() {
        let dir = std::env::temp_dir().join(format!("hex-shot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one.png");
        write_png(&path, 2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(&encode_png(1, 1, &[0, 0, 0, 255]).unwrap()[1..4], b"PNG");
    }

    #[test]
    fn a_large_shot_is_fitted_and_a_small_one_kept() {
        let rgba = vec![200_u8; 400 * 100 * 4];
        let (w, h, out) = fit(&rgba, 400, 100, 200);
        assert_eq!((w, h), (200, 50));
        assert_eq!(out.len(), 200 * 50 * 4);
        assert!(out.iter().all(|byte| *byte == 200));
        let (w, h, _) = fit(&rgba, 400, 100, 1000);
        assert_eq!((w, h), (400, 100));
    }
}
