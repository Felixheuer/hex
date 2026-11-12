//! The agents' cursor: a soft, rounded arrow in a task's colour with a
//! white rim and a coloured glow, so a cursor moving on the desktop matches
//! its task's outline and dot. Drawn once per colour with Rune offscreen
//! into `<support>/assistant/cursors/` the first time one is asked for,
//! for agent-desktop's `cursor-overlay enable --image`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rune::text::TextSystem;
use rune::{Cx, El, Env, LineCap, Shape, canvas, el};

use super::palette::{self, COLOURS};

/// The image's size in points; drawn at 2×.
pub(crate) const SIZE: (f32, f32) = (40.0, 40.0);
/// Bumped when the drawing changes, so older files are drawn again.
const VERSION: u32 = 1;
/// The arrow on a 24-point grid, tip first, then scaled into [`SIZE`].
const ARROW: [[f32; 2]; 7] = [
    [0.0, 0.0],
    [0.0, 20.0],
    [5.2, 15.4],
    [8.6, 23.2],
    [12.0, 21.7],
    [8.7, 14.2],
    [15.6, 14.2],
];
/// Where the tip sits in the image, leaving room for the glow.
const TIP: [f32; 2] = [9.0, 7.0];
const SCALE: f32 = 1.15;

/// Serialises drawing, so two tasks starting at once draw each file once.
static DRAWING: Mutex<()> = Mutex::new(());

/// Task `colour`'s cursor image (a PNG, the arrow's tip at
/// [`cursor_hotspot`]), drawn on first use; `None` when it can't be drawn
/// (no GPU, no support folder).
pub(crate) fn cursor_image(colour: u8) -> Option<PathBuf> {
    let index = colour % COLOURS;
    let dir = crate::assistant::store::assistant_dir()
        .ok()?
        .join("cursors");
    let path = file(&dir, index);
    if path.exists() {
        return Some(path);
    }
    let _drawing = DRAWING.lock().unwrap_or_else(|error| error.into_inner());
    if !path.exists()
        && let Err(error) = draw_all(&dir)
    {
        tracing::warn!(%error, "assistant: couldn't draw the agents' cursors");
    }
    path.exists().then_some(path)
}

fn file(dir: &Path, index: u8) -> PathBuf {
    dir.join(format!("cursor-{index}-v{VERSION}.png"))
}

fn draw_all(dir: &Path) -> Result<(), String> {
    let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None)?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    for index in 0..COLOURS {
        let pixels = render(&gpu, index);
        let path = file(dir, index);
        let partial = path.with_extension("png.partial");
        let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
        rune::render::write_png(&partial, w, h, &pixels).map_err(|error| error.to_string())?;
        std::fs::rename(&partial, &path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Cursor `index` drawn on a clear background: its RGBA.
pub(crate) fn render(gpu: &rune::render::Gpu, index: u8) -> Vec<u8> {
    let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
    let mut rt = crate::rune_ui::offscreen::runtime(
        index,
        view,
        SIZE,
        Env {
            system_dark: false,
            scale: 2.0,
        },
        TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
    );
    rt.settle_all();
    rt.frame(0.0);
    let mut renderer = crate::rune_ui::offscreen::renderer(&gpu.device);
    let target = crate::rune_ui::offscreen::target(&gpu.device, w, h);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (w, h),
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut encoder, &target.view, [0.0, 0.0, 0.0, 0.0]);
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    target.read(&gpu.device)
}

/// The arrow's corners in the image.
fn arrow() -> Vec<[f32; 2]> {
    ARROW
        .iter()
        .map(|[x, y]| [TIP[0] + x * SCALE, TIP[1] + y * SCALE])
        .collect()
}

pub(crate) fn view(index: &u8, _: &mut Cx<u8>) -> El {
    // The light-desktop shade: deep enough to read on any window.
    let colour = palette::task(*index, false);
    let glow = el()
        .key("cursor-glow")
        .absolute(TIP[0] + 2.0, TIP[1] + 6.0)
        .size(12.0, 14.0)
        .radius(7.0)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 12.0,
            spread: 2.0,
            color: colour.alpha(0.55),
            inset: false,
        });
    let points = arrow();
    let body = Shape::new().fill(points.clone(), colour).stroke_with(
        points,
        true,
        2.0,
        LineCap::Round,
        rune::Color::WHITE,
    );
    el().key("cursor").size(SIZE.0, SIZE.1).child(glow).child(
        canvas(body)
            .key("cursor-arrow")
            .absolute(0.0, 0.0)
            .size(SIZE.0, SIZE.1),
    )
}
