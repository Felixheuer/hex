//! Drawing on images: boxes, arrows, highlights, and text (a system font
//! through ab_glyph), anti-aliased with tiny-skia. `hex_image` annotates
//! with it and `hex_video_make` draws its text overlays and title cards, so
//! no ffmpeg build needs its own text filter.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use image::RgbaImage;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

/// Fonts tried, in order: macOS, then Linux and Windows. The first two
/// macOS ones read most scripts.
const FONTS: &[(&str, u32)] = &[
    ("/System/Library/Fonts/Supplemental/Arial Unicode.ttf", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
    ("/System/Library/Fonts/Supplemental/Arial.ttf", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
    ("C:\\Windows\\Fonts\\arial.ttf", 0),
];
const BOLD_FONTS: &[(&str, u32)] = &[
    ("/System/Library/Fonts/Supplemental/Arial Bold.ttf", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 1),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 0),
    ("C:\\Windows\\Fonts\\arialbd.ttf", 0),
];

fn load(list: &[(&str, u32)]) -> Option<FontVec> {
    list.iter().find_map(|(path, index)| {
        let bytes = std::fs::read(path).ok()?;
        FontVec::try_from_vec_and_index(bytes, *index).ok()
    })
}

/// The text font, regular or bold (falling back to regular).
pub fn font(bold: bool) -> Result<&'static FontVec, String> {
    static REGULAR: std::sync::OnceLock<Option<FontVec>> = std::sync::OnceLock::new();
    static BOLD: std::sync::OnceLock<Option<FontVec>> = std::sync::OnceLock::new();
    let regular = || REGULAR.get_or_init(|| load(FONTS)).as_ref();
    let found = if bold {
        BOLD.get_or_init(|| load(BOLD_FONTS))
            .as_ref()
            .or_else(regular)
    } else {
        regular()
    };
    found.ok_or_else(|| "no system font to draw text with".to_owned())
}

/// A color by name (`red`, `yellow`, …) or `#rrggbb[aa]`.
pub fn color(name: &str) -> Option<[u8; 4]> {
    let name = name.trim().to_ascii_lowercase();
    if let Some(hex) = name.strip_prefix('#') {
        let value = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        return match hex.len() {
            6 => Some([value(0)?, value(2)?, value(4)?, 255]),
            8 => Some([value(0)?, value(2)?, value(4)?, value(6)?]),
            _ => None,
        };
    }
    Some(match name.as_str() {
        "red" => [230, 40, 40, 255],
        "green" => [30, 170, 70, 255],
        "blue" => [40, 110, 230, 255],
        "yellow" => [255, 210, 0, 255],
        "orange" => [255, 140, 0, 255],
        "purple" | "magenta" => [170, 60, 200, 255],
        "cyan" => [0, 190, 220, 255],
        "white" => [255, 255, 255, 255],
        "black" => [0, 0, 0, 255],
        "gray" | "grey" => [128, 128, 128, 255],
        "transparent" | "none" => [0, 0, 0, 0],
        _ => return None,
    })
}

fn paint(rgba: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
    paint.anti_alias = true;
    paint
}

/// A canvas over an image (premultiplied, as tiny-skia draws).
pub struct Canvas {
    pub pixmap: Pixmap,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        Pixmap::new(width.max(1), height.max(1))
            .map(|pixmap| Self { pixmap })
            .ok_or_else(|| format!("can't draw a {width}×{height} image"))
    }

    pub fn from_image(image: &RgbaImage) -> Result<Self, String> {
        let mut canvas = Self::new(image.width(), image.height())?;
        for (pixel, out) in image.pixels().zip(canvas.pixmap.pixels_mut()) {
            let [r, g, b, a] = pixel.0;
            *out = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
        }
        Ok(canvas)
    }

    pub fn into_image(self) -> RgbaImage {
        let (width, height) = (self.pixmap.width(), self.pixmap.height());
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for pixel in self.pixmap.pixels() {
            let color = pixel.demultiply();
            data.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        RgbaImage::from_raw(width, height, data).unwrap_or_else(|| RgbaImage::new(1, 1))
    }

    /// A filled rectangle.
    pub fn fill(&mut self, x: f32, y: f32, width: f32, height: f32, rgba: [u8; 4]) {
        if let Some(rect) = Rect::from_xywh(x, y, width, height) {
            self.pixmap
                .fill_rect(rect, &paint(rgba), Transform::identity(), None);
        }
    }

    /// A rectangle's outline, `line` wide.
    pub fn frame(&mut self, x: f32, y: f32, width: f32, height: f32, rgba: [u8; 4], line: f32) {
        let Some(rect) = Rect::from_xywh(x, y, width, height) else {
            return;
        };
        let path = PathBuilder::from_rect(rect);
        let stroke = Stroke {
            width: line,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(&path, &paint(rgba), &stroke, Transform::identity(), None);
    }

    /// An arrow from `from` to `to`, its head at `to`.
    pub fn arrow(&mut self, from: (f32, f32), to: (f32, f32), rgba: [u8; 4], line: f32) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1.0 {
            return;
        }
        let (ux, uy) = (dx / length, dy / length);
        let head = (line * 4.5).max(12.0).min(length * 0.6);
        let base = (to.0 - ux * head, to.1 - uy * head);
        let mut shaft = PathBuilder::new();
        shaft.move_to(from.0, from.1);
        shaft.line_to(base.0, base.1);
        let stroke = Stroke {
            width: line,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        if let Some(path) = shaft.finish() {
            self.pixmap
                .stroke_path(&path, &paint(rgba), &stroke, Transform::identity(), None);
        }
        let half = head * 0.55;
        let mut tip = PathBuilder::new();
        tip.move_to(to.0, to.1);
        tip.line_to(base.0 - uy * half, base.1 + ux * half);
        tip.line_to(base.0 + uy * half, base.1 - ux * half);
        tip.close();
        if let Some(path) = tip.finish() {
            self.pixmap.fill_path(
                &path,
                &paint(rgba),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// `text` at `size` px with its top left at `(x, y)`, wrapped at
    /// `max_width`; returns the size it took.
    pub fn text(
        &mut self,
        text: &str,
        (x, y): (f32, f32),
        size: f32,
        rgba: [u8; 4],
        bold: bool,
        max_width: f32,
    ) -> Result<(f32, f32), String> {
        let font = font(bold)?;
        let lines = wrap(font, size, text, max_width);
        let scaled = font.as_scaled(PxScale::from(size));
        let line_height = scaled.height() + scaled.line_gap();
        let mut widest: f32 = 0.0;
        for (index, line) in lines.iter().enumerate() {
            let baseline = y + scaled.ascent() + index as f32 * line_height;
            widest = widest.max(self.line(font, size, line, (x, baseline), rgba));
        }
        Ok((widest, line_height * lines.len() as f32))
    }

    fn line(
        &mut self,
        font: &FontVec,
        size: f32,
        line: &str,
        at: (f32, f32),
        rgba: [u8; 4],
    ) -> f32 {
        let scaled = font.as_scaled(PxScale::from(size));
        let (width, height) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        let mut caret = at.0;
        let mut previous = None;
        for c in line.chars() {
            let id = font.glyph_id(c);
            if let Some(before) = previous {
                caret += scaled.kern(before, id);
            }
            let glyph =
                id.with_scale_and_position(PxScale::from(size), ab_glyph::point(caret, at.1));
            caret += scaled.h_advance(id);
            previous = Some(id);
            let Some(outlined) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            let pixels = self.pixmap.pixels_mut();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= width || py >= height {
                    return;
                }
                let alpha = coverage.clamp(0.0, 1.0) * f32::from(rgba[3]) / 255.0;
                let pixel = &mut pixels[(py * width + px) as usize];
                let blend = |under: u8, over: u8| {
                    (f32::from(over) * alpha + f32::from(under) * (1.0 - alpha)).round() as u8
                };
                // Premultiplied "over": source colors carry their alpha.
                let red = blend(pixel.red(), rgba[0]);
                let green = blend(pixel.green(), rgba[1]);
                let blue = blend(pixel.blue(), rgba[2]);
                let a = (alpha * 255.0 + f32::from(pixel.alpha()) * (1.0 - alpha)).round() as u8;
                if let Some(color) = tiny_skia::PremultipliedColorU8::from_rgba(
                    red.min(a),
                    green.min(a),
                    blue.min(a),
                    a,
                ) {
                    *pixel = color;
                }
            });
        }
        caret - at.0
    }
}

/// How wide `text` is at `size` px.
pub fn width_of(font: &FontVec, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut previous = None;
    let mut width = 0.0;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(before) = previous {
            width += scaled.kern(before, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// `text`'s lines at `size` px, words wrapped at `max_width` (and at its
/// own line breaks).
pub fn wrap(font: &FontVec, size: f32, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && width_of(font, size, &candidate) > max_width {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The size `text` takes at `size` px wrapped at `max_width`.
pub fn measure(text: &str, size: f32, bold: bool, max_width: f32) -> Result<(f32, f32), String> {
    let font = font(bold)?;
    let lines = wrap(font, size, text, max_width);
    let scaled = font.as_scaled(PxScale::from(size));
    let widest = lines
        .iter()
        .map(|line| width_of(font, size, line))
        .fold(0.0, f32::max);
    Ok((
        widest,
        (scaled.height() + scaled.line_gap()) * lines.len() as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_shapes_and_text_land_on_the_image() {
        assert_eq!(color("#ff000080"), Some([255, 0, 0, 128]));
        assert_eq!(color("Red"), Some([230, 40, 40, 255]));
        assert_eq!(color("#12"), None);
        assert_eq!(color("plaid"), None);
        let mut canvas = Canvas::from_image(&RgbaImage::from_pixel(
            200,
            100,
            image::Rgba([255, 255, 255, 255]),
        ))
        .unwrap();
        canvas.frame(10.0, 10.0, 50.0, 50.0, [255, 0, 0, 255], 4.0);
        canvas.arrow((100.0, 80.0), (180.0, 20.0), [0, 0, 255, 255], 3.0);
        canvas.fill(0.0, 90.0, 200.0, 10.0, [0, 0, 0, 255]);
        let drawn = canvas.text("Hallo Ü", (70.0, 10.0), 20.0, [0, 0, 0, 255], true, 200.0);
        let image = canvas.into_image();
        assert_eq!(image.get_pixel(10, 30).0[..3], [255, 0, 0]);
        assert_eq!(image.get_pixel(100, 95).0[..3], [0, 0, 0]);
        assert_eq!(image.get_pixel(150, 50).0[..3], [255, 255, 255]);
        if let Ok((width, height)) = drawn {
            assert!(width > 40.0 && height > 15.0, "{width}×{height}");
            let dark = (70..150)
                .flat_map(|x| (10..35).map(move |y| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0[0] < 100)
                .count();
            assert!(dark > 30, "{dark} dark pixels of text");
            let font = font(false).unwrap();
            assert_eq!(wrap(font, 20.0, "eins zwei drei vier", 30.0).len(), 4);
            assert_eq!(wrap(font, 20.0, "a\nb", 600.0), ["a", "b"]);
        }
    }
}
