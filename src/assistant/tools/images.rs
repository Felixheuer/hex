//! `hex_compare_images`: how alike two screenshots are, for a job's own
//! check ("is my page pixel-identical to the reference?"). Besides a quick
//! similarity on a small grid, the comparison runs at full resolution: the
//! second image is scaled to the first's size when their shapes match (a
//! Retina and a normal capture of the same viewport), else the shared
//! top-left area is compared and the size mismatch reported. A pixel differs
//! when any channel differs by more than `tolerance`; the answer is the
//! share of differing pixels, whether that is within the job's `threshold`,
//! the regions that differ most (fractions of the image), and, with `diff`,
//! a PNG written there showing the reference dimmed and every differing
//! pixel in red.

use serde_json::{Value, json};

/// The long side both images are compared at.
const SIDE: usize = 192;
/// The grid the differing regions are told in.
const CELLS: usize = 8;
/// A cell differs when its pixels differ by this much on average (0..255).
const CELL_DIFFERS: f64 = 24.0;

/// An image as RGBA.
struct Image {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

/// How a comparison is judged.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Per-channel difference (0..255) a pixel may have and still match.
    pub tolerance: u8,
    /// The share of differing pixels (percent) still called identical.
    pub threshold: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance: 16,
            threshold: 0.5,
        }
    }
}

/// The tool's answer for the files at `a` and `b`; with `diff`, the diff
/// image is written there.
pub fn compare(a: &str, b: &str, options: Options, diff: Option<&str>) -> Result<String, String> {
    let first = load(a)?;
    let second = load(b)?;
    let mut answer = report(&first, &second);
    let exact = pixel_diff(&first, &second, options);
    if let Some(path) = diff {
        let png = encode_png(&exact.image).ok_or("Couldn't encode the diff image.")?;
        std::fs::write(path, png).map_err(|error| format!("Couldn't write {path}: {error}"))?;
        answer["diff_image"] = json!(path);
    }
    answer["pixels"] = exact.summary;
    Ok(answer.to_string())
}

/// What the full-resolution comparison found.
struct PixelDiff {
    summary: Value,
    image: Image,
}

/// The cells the full-resolution regions are told in, per side.
const DIFF_CELLS: usize = 16;

/// Compares `a` and `b` pixel by pixel at `a`'s resolution.
fn pixel_diff(a: &Image, b: &Image, options: Options) -> PixelDiff {
    let aspect = |image: &Image| image.width as f64 / image.height.max(1) as f64;
    let same_shape = (aspect(a) - aspect(b)).abs() < 0.02;
    // Same shape: `b` scaled to `a`'s size. Else the shared top-left area.
    let (width, height) = if same_shape {
        (a.width, a.height)
    } else {
        (a.width.min(b.width), a.height.min(b.height))
    };
    let at_b = |x: usize, y: usize| -> usize {
        let (bx, by) = if same_shape {
            (
                (x * b.width / a.width.max(1)).min(b.width.saturating_sub(1)),
                (y * b.height / a.height.max(1)).min(b.height.saturating_sub(1)),
            )
        } else {
            (x, y)
        };
        (by * b.width + bx) * 4
    };
    let mut rgba = Vec::with_capacity(width * height * 4);
    let mut differing = 0usize;
    let mut cells = vec![0usize; DIFF_CELLS * DIFF_CELLS];
    let mut cell_sizes = vec![0usize; DIFF_CELLS * DIFF_CELLS];
    for y in 0..height {
        for x in 0..width {
            let ia = (y * a.width + x) * 4;
            let ib = at_b(x, y);
            let over = (0..3).any(|c| a.rgba[ia + c].abs_diff(b.rgba[ib + c]) > options.tolerance);
            let cell =
                (y * DIFF_CELLS / height.max(1)) * DIFF_CELLS + x * DIFF_CELLS / width.max(1);
            cell_sizes[cell] += 1;
            if over {
                differing += 1;
                cells[cell] += 1;
                rgba.extend([255, 0, 0, 255]);
            } else {
                // The reference, dimmed toward white so the red stands out.
                rgba.extend((0..3).map(|c| 170 + a.rgba[ia + c] / 3));
                rgba.push(255);
            }
        }
    }
    let total = (width * height).max(1);
    let percent = differing as f64 * 100.0 / total as f64;
    let step = 1.0 / DIFF_CELLS as f64;
    let mut regions: Vec<(usize, f64)> = cells
        .iter()
        .zip(&cell_sizes)
        .enumerate()
        .filter(|(_, (count, size))| **count > 0 && **size > 0)
        .map(|(index, (count, size))| (index, *count as f64 * 100.0 / *size as f64))
        .collect();
    regions.sort_by(|x, y| y.1.total_cmp(&x.1));
    let regions: Vec<Value> = regions
        .iter()
        .take(12)
        .map(|(index, share)| {
            json!({
                "x": (index % DIFF_CELLS) as f64 * step,
                "y": (index / DIFF_CELLS) as f64 * step,
                "width": step,
                "height": step,
                "differing_percent": (share * 10.0).round() / 10.0,
            })
        })
        .collect();
    let mut summary = json!({
        "compared_at": [width, height],
        "differing_pixels": differing,
        "differing_percent": (percent * 1000.0).round() / 1000.0,
        "tolerance": options.tolerance,
        "threshold_percent": options.threshold,
        "identical": same_shape && percent <= options.threshold,
        "differing_regions": regions,
    });
    if !same_shape {
        summary["size_mismatch"] = json!(format!(
            "{}x{} vs {}x{}: only the shared top-left area was compared; capture both at the same viewport size",
            a.width, a.height, b.width, b.height
        ));
    }
    PixelDiff {
        summary,
        image: Image {
            width,
            height,
            rgba,
        },
    }
}

fn encode_png(image: &Image) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(
            &mut bytes,
            u32::try_from(image.width).ok()?,
            u32::try_from(image.height).ok()?,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&image.rgba).ok()?;
    }
    Some(bytes)
}

fn load(path: &str) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Couldn't read {path}: {error}"))?;
    decode(&bytes).ok_or_else(|| format!("{path} isn't a PNG or JPEG image Hex can read."))
}

fn decode(bytes: &[u8]) -> Option<Image> {
    if bytes.starts_with(b"\x89PNG") {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(bytes)
    } else {
        None
    }
}

fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[value, alpha]| [*value, *value, *value, *alpha])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image {
        width: usize::try_from(info.width).ok()?,
        height: usize::try_from(info.height).ok()?,
        rgba,
    })
}

fn decode_jpeg(bytes: &[u8]) -> Option<Image> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder =
        zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
    let rgba = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    (rgba.len() == width * height * 4).then_some(Image {
        width,
        height,
        rgba,
    })
}

/// `image` sampled to `width` × `height` (nearest pixel, alpha over white).
fn sample(image: &Image, width: usize, height: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let sx = (x * image.width / width).min(image.width.saturating_sub(1));
            let sy = (y * image.height / height).min(image.height.saturating_sub(1));
            let at = (sy * image.width + sx) * 4;
            let alpha = f64::from(image.rgba[at + 3]) / 255.0;
            let channel = |c: usize| f64::from(image.rgba[at + c]) * alpha + 255.0 * (1.0 - alpha);
            out.push([channel(0), channel(1), channel(2)]);
        }
    }
    out
}

fn report(a: &Image, b: &Image) -> Value {
    let (width, height) = if a.width >= a.height {
        (SIDE, (SIDE * a.height / a.width.max(1)).max(CELLS))
    } else {
        ((SIDE * a.width / a.height.max(1)).max(CELLS), SIDE)
    };
    let (pa, pb) = (sample(a, width, height), sample(b, width, height));
    let mut cells = vec![(0.0f64, 0usize); CELLS * CELLS];
    let mut total = 0.0;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let diff = (0..3).map(|c| (pa[i][c] - pb[i][c]).abs()).sum::<f64>() / 3.0;
            total += diff;
            let cell = (y * CELLS / height) * CELLS + x * CELLS / width;
            cells[cell].0 += diff;
            cells[cell].1 += 1;
        }
    }
    let mean = total / (width * height) as f64;
    let similarity = (100.0 * (1.0 - mean / 255.0)).clamp(0.0, 100.0);
    let mut differing: Vec<(usize, f64)> = cells
        .iter()
        .enumerate()
        .filter(|(_, (_, n))| *n > 0)
        .map(|(index, (sum, n))| (index, sum / *n as f64))
        .filter(|(_, diff)| *diff >= CELL_DIFFERS)
        .collect();
    differing.sort_by(|x, y| y.1.total_cmp(&x.1));
    let step = 1.0 / CELLS as f64;
    let regions: Vec<Value> = differing
        .iter()
        .take(12)
        .map(|(index, diff)| {
            json!({
                "x": (index % CELLS) as f64 * step,
                "y": (index / CELLS) as f64 * step,
                "width": step,
                "height": step,
                "difference": (diff / 255.0 * 100.0).round(),
            })
        })
        .collect();
    let aspect = |image: &Image| image.width as f64 / image.height.max(1) as f64;
    json!({
        "similarity": (similarity * 10.0).round() / 10.0,
        "sizes": [[a.width, a.height], [b.width, b.height]],
        "same_shape": (aspect(a) - aspect(b)).abs() < 0.02,
        "differing_regions": regions,
        "note": "Regions are fractions of the image (x, y from the top left). Compare at the same window size for a fair score; 95+ is near identical, below 80 looks clearly different."
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 4]) -> Image {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend(paint(x, y));
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    #[test]
    fn alike_images_score_high_and_a_changed_corner_is_named() {
        let white = image(400, 300, |_, _| [255, 255, 255, 255]);
        let same = report(&white, &image(800, 600, |_, _| [255, 255, 255, 255]));
        assert_eq!(same["similarity"], 100.0);
        assert!(same["differing_regions"].as_array().unwrap().is_empty());
        assert_eq!(same["same_shape"], true);
        let corner = image(400, 300, |x, y| {
            if x >= 350 && y < 37 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let changed = report(&white, &corner);
        let similarity = changed["similarity"].as_f64().unwrap();
        assert!(similarity > 95.0 && similarity < 100.0, "{similarity}");
        let first = &changed["differing_regions"][0];
        assert_eq!(
            (first["x"].as_f64(), first["y"].as_f64()),
            (Some(0.875), Some(0.0))
        );
        let black = image(400, 300, |_, _| [0, 0, 0, 255]);
        assert!(report(&white, &black)["similarity"].as_f64().unwrap() < 1.0);
    }

    #[test]
    fn pngs_and_jpegs_are_read_and_other_files_refused() {
        let jpeg = include_bytes!("../../../tests/fixtures/live-frame.jpg");
        let image = decode(jpeg).expect("the fixture JPEG decodes");
        assert!(image.width > 0 && image.rgba.len() == image.width * image.height * 4);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let image = decode(&png).expect("the PNG decodes");
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(&image.rgba[..4], [255, 0, 0, 255]);
        assert!(decode(b"GIF89a").is_none());
        assert!(
            compare(
                "/nonexistent/a.png",
                "/nonexistent/b.png",
                Options::default(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn full_resolution_diff_counts_pixels_and_draws_them_red() {
        let white = image(200, 100, |_, _| [255, 255, 255, 255]);
        // Identical, and a Retina capture of the same viewport too.
        let retina = image(400, 200, |_, _| [255, 255, 255, 255]);
        let same = pixel_diff(&white, &retina, Options::default());
        assert_eq!(same.summary["differing_pixels"], 0);
        assert_eq!(same.summary["identical"], true);
        // Ten pixels off: 0.05 %, within the default threshold; a strict one says no.
        let dots = image(200, 100, |x, y| {
            if y == 0 && x < 10 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let few = pixel_diff(&white, &dots, Options::default());
        assert_eq!(few.summary["differing_pixels"], 10);
        assert_eq!(few.summary["identical"], true);
        let strict = pixel_diff(
            &white,
            &dots,
            Options {
                tolerance: 16,
                threshold: 0.0,
            },
        );
        assert_eq!(strict.summary["identical"], false);
        assert_eq!(
            &strict.image.rgba[..4],
            [255, 0, 0, 255],
            "a differing pixel is red"
        );
        assert_ne!(
            &strict.image.rgba[40..44],
            [255, 0, 0, 255],
            "a matching one isn't"
        );
        // Within tolerance (a slightly different white) counts as the same.
        let off_white = image(200, 100, |_, _| [250, 250, 250, 255]);
        let close = pixel_diff(&white, &off_white, Options::default());
        assert_eq!(close.summary["differing_pixels"], 0);
        // Different shapes: the shared area, never "identical".
        let tall = image(200, 300, |_, _| [255, 255, 255, 255]);
        let mismatch = pixel_diff(&white, &tall, Options::default());
        assert_eq!(mismatch.summary["identical"], false);
        assert!(mismatch.summary["size_mismatch"].is_string());
        assert!(encode_png(&strict.image).unwrap().starts_with(b"\x89PNG"));
    }
}
