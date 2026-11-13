//! `hex_image`: look at and edit an image: inspect it (size, format,
//! colors), crop, resize, rotate, annotate it (boxes, arrows, highlights,
//! labels), convert it to another format, compare it with another image
//! (`hex_compare_images`' check, with its diff image), or read its text
//! (Vision OCR). The result is written into the task's folder and shown.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use serde_json::{Value, json};

use super::draw::{self, Canvas};
use super::out;

pub const NAME: &str = "hex_image";

/// The largest image edited, in pixels.
const MAX_PIXELS: u64 = 100_000_000;

pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "Look at or edit an image file (PNG, JPEG, HEIC, WebP, GIF, TIFF, BMP) and see the result: op inspect (size, format, main colors), crop (x, y, width, height in pixels, or fractions 0-1), resize (width and/or height, or scale), rotate (degrees 90/180/270, flip), annotate (shapes: box, arrow, highlight, text; colors by name or #hex; coordinates in pixels), convert (to the out file's format; quality for JPEG), compare (with `other`: similarity, differing regions, a diff image), text (OCR: the text with each line's box). Edits are written into your folder (or `out`) and shown.",
        "inputSchema": {"type": "object", "properties": {
            "op": {"type": "string", "enum": ["inspect", "crop", "resize", "rotate", "annotate", "convert", "compare", "text"]},
            "path": {"type": "string", "description": "The image's path."},
            "out": {"type": "string", "description": "Where to write the result (default: a new file in your folder)."},
            "x": {"type": "number"}, "y": {"type": "number"}, "width": {"type": "number"}, "height": {"type": "number"},
            "scale": {"type": "number", "description": "resize: a factor, e.g. 0.5."},
            "degrees": {"type": "integer", "enum": [90, 180, 270]},
            "flip": {"type": "string", "enum": ["horizontal", "vertical"]},
            "shapes": {"type": "array", "description": "annotate: [{\"type\": \"box\"|\"arrow\"|\"highlight\"|\"text\", \"x\", \"y\", \"width\", \"height\" (box/highlight), \"from\": [x, y], \"to\": [x, y] (arrow), \"text\" (a label on a box, or the text), \"color\", \"size\" (text px), \"line\" (px)}]", "items": {"type": "object"}},
            "quality": {"type": "integer", "minimum": 1, "maximum": 100},
            "other": {"type": "string", "description": "compare: the second image."},
            "tolerance": {"type": "integer"}, "threshold": {"type": "number"}
        }, "required": ["op", "path"]},
    })
}

/// The image at `path`: decoded here, or (HEIC and other formats macOS
/// reads) converted by `sips` first.
pub fn load(path: &Path, scratch: &Path) -> Result<DynamicImage, String> {
    let direct = image::ImageReader::open(path)
        .map_err(|error| error.to_string())
        .and_then(|reader| {
            reader
                .with_guessed_format()
                .map_err(|error| error.to_string())
        })
        .and_then(|reader| {
            if let Ok((width, height)) = reader.into_dimensions()
                && u64::from(width) * u64::from(height) > MAX_PIXELS
            {
                return Err(format!("the image is too large ({width}×{height})"));
            }
            image::open(path).map_err(|error| error.to_string())
        });
    match direct {
        Ok(image) => Ok(image),
        Err(error) if error.contains("too large") => Err(error),
        Err(error) if cfg!(target_os = "macos") => {
            let png = scratch.join(format!(".sips-{}.png", std::process::id()));
            let mut command = Command::new("/usr/bin/sips");
            command
                .args(["-s", "format", "png"])
                .arg(path)
                .arg("--out")
                .arg(&png);
            out::run(command, Duration::from_secs(60), "sips")
                .map_err(|_| format!("Couldn't read {} as an image: {error}", path.display()))?;
            let image = image::open(&png).map_err(|error| error.to_string());
            let _ = std::fs::remove_file(&png);
            image
        }
        Err(error) => Err(format!(
            "Couldn't read {} as an image: {error}",
            path.display()
        )),
    }
}

/// Writes `image` to `path` in its extension's format.
pub fn save(image: &DynamicImage, path: &Path, quality: u8) -> Result<(), String> {
    let extension = out::extension(path);
    let failed = |error: image::ImageError| format!("Couldn't write {}: {error}", path.display());
    match extension.as_str() {
        "jpg" | "jpeg" => {
            let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
            let mut writer = std::io::BufWriter::new(file);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality)
                .encode_image(&image.to_rgb8())
                .map_err(failed)
        }
        "heic" | "heif" if cfg!(target_os = "macos") => {
            let png = path.with_extension("tmp.png");
            image
                .save_with_format(&png, ImageFormat::Png)
                .map_err(failed)?;
            let mut command = Command::new("/usr/bin/sips");
            command
                .args(["-s", "format", "heic"])
                .arg(&png)
                .arg("--out")
                .arg(path);
            let converted = out::run(command, Duration::from_secs(60), "sips");
            let _ = std::fs::remove_file(&png);
            converted.map(|_| ())
        }
        _ => {
            let format = ImageFormat::from_extension(&extension)
                .ok_or_else(|| format!("“.{extension}” isn't an image format to write (png, jpg, webp, gif, tiff, bmp, heic)."))?;
            match format {
                // Without an alpha channel in these.
                ImageFormat::Bmp => {
                    DynamicImage::ImageRgb8(image.to_rgb8()).save_with_format(path, format)
                }
                _ => image.save_with_format(path, format),
            }
            .map_err(failed)
        }
    }
}

fn number(arguments: &Value, key: &str) -> Option<f64> {
    arguments[key].as_f64().filter(|value| value.is_finite())
}

/// A region from x, y, width, height: pixels, or fractions when all ≤ 1.
fn region(arguments: &Value, (width, height): (u32, u32)) -> Result<(u32, u32, u32, u32), String> {
    let values = ["x", "y", "width", "height"].map(|key| number(arguments, key));
    let [Some(x), Some(y), Some(w), Some(h)] = values else {
        return Err("crop needs x, y, width, and height".into());
    };
    let fractions = [x, y, w, h].iter().all(|value| (0.0..=1.0).contains(value))
        && (w < 1.0 || h < 1.0 || x > 0.0 || y > 0.0);
    let (x, y, w, h) = if fractions {
        (
            x * f64::from(width),
            y * f64::from(height),
            w * f64::from(width),
            h * f64::from(height),
        )
    } else {
        (x, y, w, h)
    };
    let x = (x.max(0.0) as u32).min(width.saturating_sub(1));
    let y = (y.max(0.0) as u32).min(height.saturating_sub(1));
    let w = (w.max(1.0) as u32).min(width - x);
    let h = (h.max(1.0) as u32).min(height - y);
    Ok((x, y, w, h))
}

/// The image's main colors: up to five, with their share.
fn colors(image: &DynamicImage) -> Vec<Value> {
    let small = image.thumbnail(64, 64).to_rgba8();
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    let mut total = 0;
    for pixel in small.pixels() {
        if pixel.0[3] < 128 {
            continue;
        }
        let key = [pixel.0[0] & 0xE0, pixel.0[1] & 0xE0, pixel.0[2] & 0xE0];
        *counts.entry(key).or_default() += 1;
        total += 1;
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    sorted
        .into_iter()
        .take(5)
        .map(|(color, count)| {
            let hex = format!("#{:02x}{:02x}{:02x}", color[0] | 0x10, color[1] | 0x10, color[2] | 0x10);
            json!({"color": hex, "share": ((count as f64 / total.max(1) as f64) * 100.0).round() / 100.0})
        })
        .collect()
}

fn color_of(shape: &Value, default: [u8; 4]) -> Result<[u8; 4], String> {
    match shape["color"].as_str() {
        Some(name) => {
            draw::color(name).ok_or_else(|| format!("“{name}” isn't a color (a name or #rrggbb)"))
        }
        None => Ok(default),
    }
}

fn point(value: &Value) -> Option<(f32, f32)> {
    let pair = value.as_array()?;
    Some((
        pair.first()?.as_f64()? as f32,
        pair.get(1)?.as_f64()? as f32,
    ))
}

/// Draws `shapes` on `image`.
pub fn annotate(image: &DynamicImage, shapes: &[Value]) -> Result<RgbaImage, String> {
    let mut canvas = Canvas::from_image(&image.to_rgba8())?;
    let edge = image.width().max(image.height()) as f32;
    let default_line = (edge / 300.0).clamp(2.0, 12.0);
    let default_size = (edge / 40.0).clamp(14.0, 64.0);
    for shape in shapes {
        let number = |key: &str| shape[key].as_f64().map(|value| value as f32);
        let line = number("line").unwrap_or(default_line);
        let size = number("size").unwrap_or(default_size);
        match shape["type"].as_str().unwrap_or("box") {
            "box" | "rect" | "rectangle" | "highlight" => {
                let (Some(x), Some(y), Some(w), Some(h)) =
                    (number("x"), number("y"), number("width"), number("height"))
                else {
                    return Err("a box needs x, y, width, height".into());
                };
                if shape["type"] == "highlight" {
                    let mut color = color_of(shape, [255, 230, 0, 255])?;
                    color[3] = color[3].min(90);
                    canvas.fill(x, y, w, h, color);
                } else {
                    canvas.frame(x, y, w, h, color_of(shape, [230, 40, 40, 255])?, line);
                }
                if let Some(text) = shape["text"].as_str().filter(|text| !text.is_empty()) {
                    let color = color_of(shape, [230, 40, 40, 255])?;
                    label(
                        &mut canvas,
                        text,
                        (x, (y - size * 1.4).max(0.0)),
                        size,
                        color,
                    )?;
                }
            }
            "arrow" => {
                let (Some(from), Some(to)) = (point(&shape["from"]), point(&shape["to"])) else {
                    return Err("an arrow needs from and to, each [x, y]".into());
                };
                canvas.arrow(from, to, color_of(shape, [230, 40, 40, 255])?, line);
                if let Some(text) = shape["text"].as_str().filter(|text| !text.is_empty()) {
                    let color = color_of(shape, [230, 40, 40, 255])?;
                    label(
                        &mut canvas,
                        text,
                        (from.0, (from.1 - size * 1.4).max(0.0)),
                        size,
                        color,
                    )?;
                }
            }
            "text" | "label" => {
                let (Some(x), Some(y)) = (number("x"), number("y")) else {
                    return Err("text needs x and y".into());
                };
                let text = shape["text"].as_str().unwrap_or_default();
                label(
                    &mut canvas,
                    text,
                    (x, y),
                    size,
                    color_of(shape, [230, 40, 40, 255])?,
                )?;
            }
            other => {
                return Err(format!(
                    "“{other}” isn't a shape (box, arrow, highlight, text)"
                ));
            }
        }
    }
    Ok(canvas.into_image())
}

/// `text` in white on a `color` badge with its top left at `at`.
fn label(
    canvas: &mut Canvas,
    text: &str,
    at: (f32, f32),
    size: f32,
    color: [u8; 4],
) -> Result<(), String> {
    let width = canvas.pixmap.width() as f32;
    let (w, h) = draw::measure(text, size, true, (width - at.0 - size).max(size * 4.0))?;
    let pad = size * 0.25;
    canvas.fill(at.0, at.1, w + 2.0 * pad, h + pad, color);
    let ink = if u32::from(color[0]) + u32::from(color[1]) + u32::from(color[2]) > 600 {
        [0, 0, 0, 255]
    } else {
        [255, 255, 255, 255]
    };
    canvas.text(
        text,
        (at.0 + pad, at.1 + pad * 0.5),
        size,
        ink,
        true,
        w + 1.0,
    )?;
    Ok(())
}

pub fn call(arguments: &Value, folder: &Path) -> Result<Vec<Value>, String> {
    let started = Instant::now();
    let op = arguments["op"].as_str().unwrap_or("inspect");
    let path = out::input(arguments, "path", folder, NAME)?;
    let stem = path.file_stem().map_or_else(
        || "image".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let result_path = |suffix: &str, extension: &str| -> Result<PathBuf, String> {
        let default = out::fresh_name(folder, &format!("{stem}-{suffix}"), extension);
        out::output(arguments, "out", folder, &default)
    };
    let keep_extension = || match out::extension(&path).as_str() {
        "jpg" | "jpeg" => "jpg".to_owned(),
        "webp" => "webp".to_owned(),
        _ => "png".to_owned(),
    };
    let quality = arguments["quality"]
        .as_u64()
        .map_or(90, |quality| quality.clamp(1, 100)) as u8;
    let mut report = json!({"op": op, "source": path});
    let shown: PathBuf = match op {
        "inspect" => {
            let image = load(&path, folder)?;
            let format = image::ImageReader::open(&path)
                .ok()
                .and_then(|reader| reader.with_guessed_format().ok())
                .and_then(|reader| reader.format())
                .map_or_else(
                    || out::extension(&path),
                    |format| format!("{format:?}").to_lowercase(),
                );
            report["width"] = json!(image.width());
            report["height"] = json!(image.height());
            report["format"] = json!(format);
            report["bytes"] = json!(std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0));
            report["transparent"] = json!(
                image.color().has_alpha()
                    && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < 255)
            );
            report["colors"] = json!(colors(&image));
            path.clone()
        }
        "crop" => {
            let image = load(&path, folder)?;
            let (x, y, w, h) = region(arguments, image.dimensions())?;
            let cropped = image.crop_imm(x, y, w, h);
            let target = result_path("crop", &keep_extension())?;
            save(&cropped, &target, quality)?;
            report["region"] = json!({"x": x, "y": y, "width": w, "height": h});
            target
        }
        "resize" => {
            let image = load(&path, folder)?;
            let (width, height) = image.dimensions();
            let scale = number(arguments, "scale");
            let (w, h) = match (
                number(arguments, "width"),
                number(arguments, "height"),
                scale,
            ) {
                (_, _, Some(scale)) if scale > 0.0 => {
                    (f64::from(width) * scale, f64::from(height) * scale)
                }
                (Some(w), Some(h), _) => (w, h),
                (Some(w), None, _) => (w, w * f64::from(height) / f64::from(width)),
                (None, Some(h), _) => (h * f64::from(width) / f64::from(height), h),
                _ => return Err("resize needs width, height, or scale".into()),
            };
            let (w, h) = (
                w.round().clamp(1.0, 20_000.0) as u32,
                h.round().clamp(1.0, 20_000.0) as u32,
            );
            let resized = image.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
            let target = result_path(&format!("{w}x{h}"), &keep_extension())?;
            save(&resized, &target, quality)?;
            report["size"] = json!([w, h]);
            target
        }
        "rotate" => {
            let image = load(&path, folder)?;
            let mut turned = match arguments["degrees"].as_i64().unwrap_or(0).rem_euclid(360) {
                90 => image.rotate90(),
                180 => image.rotate180(),
                270 => image.rotate270(),
                0 => image,
                other => {
                    return Err(format!(
                        "rotate turns by 90, 180, or 270 degrees, not {other}"
                    ));
                }
            };
            match arguments["flip"].as_str() {
                Some("horizontal") => turned = turned.fliph(),
                Some("vertical") => turned = turned.flipv(),
                _ => {}
            }
            let target = result_path("rotated", &keep_extension())?;
            save(&turned, &target, quality)?;
            target
        }
        "annotate" => {
            let image = load(&path, folder)?;
            let shapes = arguments["shapes"]
                .as_array()
                .filter(|shapes| !shapes.is_empty())
                .ok_or("annotate needs shapes")?;
            let drawn = annotate(&image, shapes)?;
            let target = result_path("annotated", &keep_extension())?;
            save(&DynamicImage::ImageRgba8(drawn), &target, quality)?;
            report["shapes"] = json!(shapes.len());
            target
        }
        "convert" => {
            let image = load(&path, folder)?;
            let target =
                match arguments["out"].as_str() {
                    Some(_) => out::output(arguments, "out", folder, "converted.png")?,
                    None => return Err(
                        "convert needs out: a file name with the new extension (e.g. photo.jpg)"
                            .into(),
                    ),
                };
            save(&image, &target, quality)?;
            target
        }
        "compare" => {
            let other = out::input(arguments, "other", folder, NAME)?;
            let readable = |image: &Path, name: &str| -> Result<PathBuf, String> {
                match out::extension(image).as_str() {
                    "png" | "jpg" | "jpeg" => Ok(image.to_owned()),
                    _ => {
                        let png = folder.join(out::fresh_name(folder, name, "png"));
                        save(&load(image, folder)?, &png, 100)?;
                        Ok(png)
                    }
                }
            };
            let (a, b) = (
                readable(&path, "compare-a")?,
                readable(&other, "compare-b")?,
            );
            let diff = folder.join(out::fresh_name(folder, &format!("{stem}-diff"), "png"));
            let defaults = crate::assistant::tools::images::Options::default();
            let options = crate::assistant::tools::images::Options {
                tolerance: arguments["tolerance"]
                    .as_u64()
                    .map_or(defaults.tolerance, |value| value.min(255) as u8),
                threshold: number(arguments, "threshold")
                    .map_or(defaults.threshold, |value| value.clamp(0.0, 100.0)),
            };
            let compared = crate::assistant::tools::images::compare(
                &a.to_string_lossy(),
                &b.to_string_lossy(),
                options,
                Some(&diff.to_string_lossy()),
            )?;
            report["comparison"] = serde_json::from_str(&compared).unwrap_or(json!(compared));
            diff
        }
        "text" => {
            let read = super::document::ocr(&path)?;
            report["text"] = read["text"].clone();
            report["lines"] = read["lines"].clone();
            report["note"] = json!(
                "Line boxes are [x, y, width, height] as fractions of the image from its top left."
            );
            report["took_s"] = json!(out::took(started));
            return Ok(vec![json!({"type": "text", "text": report.to_string()})]);
        }
        other => return Err(format!("“{other}” isn't an op of {NAME}")),
    };
    if shown != path {
        report["out"] = json!(shown);
        if let Ok((width, height)) = image::image_dimensions(&shown) {
            report["size"] = json!([width, height]);
        }
    }
    report["took_s"] = json!(out::took(started));
    Ok(out::content(&report, &[shown], 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("hex-image-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn report(content: &[Value]) -> Value {
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn an_image_is_inspected_cut_marked_converted_and_compared() {
        let folder = folder("edit");
        let source = folder.join("shot.png");
        RgbaImage::from_fn(400, 300, |x, _| {
            if x < 200 {
                image::Rgba([250, 250, 250, 255])
            } else {
                image::Rgba([20, 60, 200, 255])
            }
        })
        .save(&source)
        .unwrap();
        let call = |arguments: Value| call(&arguments, &folder);
        let inspected = call(json!({"op": "inspect", "path": "shot.png"})).unwrap();
        let about = report(&inspected);
        assert_eq!(
            (about["width"].as_u64(), about["height"].as_u64()),
            (Some(400), Some(300))
        );
        assert_eq!(about["format"], "png");
        assert_eq!(about["colors"].as_array().unwrap().len(), 2);
        assert_eq!(inspected[1]["type"], "image");

        let cropped = report(&call(json!({"op": "crop", "path": "shot.png", "x": 0.5, "y": 0, "width": 0.5, "height": 0.5})).unwrap());
        assert_eq!(cropped["size"], json!([200, 150]));
        let half = image::open(cropped["out"].as_str().unwrap()).unwrap();
        assert_eq!(half.get_pixel(10, 10).0, [20, 60, 200, 255]);

        let resized =
            report(&call(json!({"op": "resize", "path": "shot.png", "width": 100})).unwrap());
        assert_eq!(resized["size"], json!([100, 75]));
        let turned =
            report(&call(json!({"op": "rotate", "path": "shot.png", "degrees": 90})).unwrap());
        assert_eq!(turned["size"], json!([300, 400]));

        let marked = report(&call(json!({"op": "annotate", "path": "shot.png", "out": "marked.png", "shapes": [
            {"type": "box", "x": 20, "y": 120, "width": 100, "height": 60, "color": "red", "text": "Hier"},
            {"type": "arrow", "from": [150, 280], "to": [230, 200], "color": "#00ff00"},
            {"type": "highlight", "x": 250, "y": 20, "width": 100, "height": 40}
        ]})).unwrap());
        let drawn = image::open(marked["out"].as_str().unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(
            drawn.get_pixel(20, 150).0[..3],
            [230, 40, 40],
            "the box's edge"
        );
        assert!(
            drawn.get_pixel(300, 40).0[0] > 20,
            "the highlight tints the blue"
        );
        assert!(
            call(json!({"op": "annotate", "path": "shot.png", "shapes": [{"type": "star"}]}))
                .is_err()
        );

        let jpeg = report(
            &call(json!({"op": "convert", "path": "shot.png", "out": "shot.jpg", "quality": 80}))
                .unwrap(),
        );
        assert_eq!(
            image::ImageReader::open(jpeg["out"].as_str().unwrap())
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .format(),
            Some(ImageFormat::Jpeg)
        );
        let webp = report(
            &call(json!({"op": "convert", "path": "shot.png", "out": "shot.webp"})).unwrap(),
        );
        assert!(Path::new(webp["out"].as_str().unwrap()).is_file());

        let same = report(
            &call(json!({"op": "compare", "path": "shot.png", "other": "shot.webp"})).unwrap(),
        );
        assert_eq!(same["comparison"]["pixels"]["identical"], true, "{same}");
        let differs =
            call(json!({"op": "compare", "path": "shot.png", "other": "marked.png"})).unwrap();
        assert_eq!(report(&differs)["comparison"]["pixels"]["identical"], false);
        assert_eq!(differs[1]["type"], "image", "the diff image is shown");
        assert!(call(json!({"op": "crop", "path": "shot.png"})).is_err());
        assert!(call(json!({"op": "fly", "path": "shot.png"})).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn text_in_an_image_is_read() {
        let folder = folder("ocr");
        let mut canvas = Canvas::from_image(&RgbaImage::from_pixel(
            900,
            200,
            image::Rgba([255, 255, 255, 255]),
        ))
        .unwrap();
        canvas
            .text(
                "Rechnung Nummer 4711",
                (40.0, 60.0),
                56.0,
                [0, 0, 0, 255],
                false,
                860.0,
            )
            .unwrap();
        canvas.into_image().save(folder.join("note.png")).unwrap();
        let read = report(&call(&json!({"op": "text", "path": "note.png"}), &folder).unwrap());
        assert!(read["text"].as_str().unwrap().contains("4711"), "{read}");
        assert_eq!(read["lines"][0]["box"].as_array().unwrap().len(), 4);
        // HEIC goes through sips both ways.
        let heic = report(
            &call(
                &json!({"op": "convert", "path": "note.png", "out": "note.heic"}),
                &folder,
            )
            .unwrap(),
        );
        let back = report(&call(&json!({"op": "inspect", "path": heic["out"]}), &folder).unwrap());
        assert_eq!(back["width"], 900);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
