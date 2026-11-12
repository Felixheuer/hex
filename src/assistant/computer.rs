//! What Hex itself reads from the desktop (macOS): a quick question's
//! context (screenshot, selected text, frontmost app) and where a task's
//! windows are, for their outlines. Tasks operate apps through agent-desktop
//! (`backends`), not through this module.
//!
//! Everything here only reads: the selection comes through Accessibility,
//! never the clipboard, and the screenshot through ScreenCaptureKit with
//! Hex's own overlays (the HUD, the frame) left out.
//!
//! OWNER: hands (with `speech`).

use color_eyre::{Result, eyre::eyre};

/// The screen as the model sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub png: Vec<u8>,
    /// Size of `png` in pixels: the coordinate space actions use.
    pub width: u32,
    pub height: u32,
}

/// A screenshot's long side, at most.
pub const LONG_SIDE: u32 = 1280;

/// The most selected text a question takes along, in characters.
const MAX_SELECTION: usize = 20_000;

/// A screenshot of the display the pointer is on, scaled so its long side
/// is at most 1280 pixels.
pub fn screenshot() -> Result<Screenshot> {
    #[cfg(target_os = "macos")]
    {
        macos::screenshot()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(eyre!("taking screenshots isn't available on this computer"))
    }
}

/// Where `window` is on screen, in global display points (x, y, width,
/// height), for its task's outline; `None` when it's gone or hidden.
///
/// A window id is agent-desktop's (`w-4521`), whose number is the
/// CoreGraphics window number; without one, the app's frontmost window.
pub fn window_frame(window: &crate::assistant::TaskWindow) -> Option<(f64, f64, f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        macos::window_frame(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}

/// What the desktop looks like now, for a question: a screenshot when
/// `with_screenshot`, the selected text, the frontmost app and its window.
/// Takes about a tenth of a second with a screenshot; off the UI thread.
pub fn context(with_screenshot: bool) -> crate::assistant::Context {
    #[cfg(target_os = "macos")]
    {
        macos::context(with_screenshot)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = with_screenshot;
        crate::assistant::Context::default()
    }
}

/// Whether Hex may read other apps and record the screen (macOS
/// Accessibility and Screen Recording), for setup to show what's missing.
/// Never blocks: Screen Recording follows a cached read that refreshes in
/// the background (see [`permissions_now`]).
pub fn permissions() -> Permissions {
    #[cfg(target_os = "macos")]
    {
        macos::permissions(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Permissions::default()
    }
}

/// [`permissions`] read afresh, so a grant made since launch shows; blocks
/// for up to a few seconds. Off the UI thread.
pub fn permissions_now() -> Permissions {
    #[cfg(target_os = "macos")]
    {
        macos::permissions(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Permissions::default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Permissions {
    pub accessibility: bool,
    pub screen_recording: bool,
}

/// One permission Hex asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Permission {
    Accessibility,
    ScreenRecording,
}

impl Permission {
    /// Its name in System Settings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "Screen Recording",
        }
    }
}

/// Opens System Settings at `permission`'s list. On the main thread Hex's
/// own guide floats beside it with Hex to drag in (as onboarding does);
/// elsewhere the list opens alone.
pub fn open_settings(permission: Permission) {
    #[cfg(target_os = "macos")]
    macos::open_settings(permission);
    #[cfg(not(target_os = "macos"))]
    let _ = permission;
}

/// The CoreGraphics window number in an agent-desktop window id (`w-4521`).
pub fn window_number(id: &str) -> Option<u32> {
    id.trim().strip_prefix("w-")?.parse().ok()
}

/// `width` × `height` scaled down so the long side is at most `long_side`,
/// keeping the aspect; unchanged when it already fits.
pub fn fit(width: u32, height: u32, long_side: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= long_side || long == 0 {
        return (width, height);
    }
    let scale = f64::from(long_side) / f64::from(long);
    let side = |pixels: u32| ((f64::from(pixels) * scale).round() as u32).clamp(1, long_side);
    (side(width), side(height))
}

/// Tightly packed RGBA resized to `to` by averaging the source pixels each
/// target pixel covers (a downscale; an upscale repeats pixels).
pub fn resize(rgba: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    let ((sw, sh), (tw, th)) = (from, to);
    if from == to {
        return rgba.to_vec();
    }
    let span = |t: u32, target: u32, source: u32| {
        let start = (u64::from(t) * u64::from(source) / u64::from(target)) as u32;
        let end = ((u64::from(t) + 1) * u64::from(source) / u64::from(target)) as u32;
        start..end.max(start + 1).min(source)
    };
    let mut out = Vec::with_capacity(tw as usize * th as usize * 4);
    for ty in 0..th {
        let rows = span(ty, th, sh);
        for tx in 0..tw {
            let columns = span(tx, tw, sw);
            let mut sum = [0_u64; 4];
            let mut count = 0_u64;
            for y in rows.clone() {
                let row = y as usize * sw as usize * 4;
                for x in columns.clone() {
                    let at = row + x as usize * 4;
                    for (total, value) in sum.iter_mut().zip(&rgba[at..at + 4]) {
                        *total += u64::from(*value);
                    }
                    count += 1;
                }
            }
            out.extend(sum.map(|total| (total / count.max(1)) as u8));
        }
    }
    out
}

/// RGBA as a PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| eyre!("Couldn't encode the screenshot: {error}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| eyre!("Couldn't encode the screenshot: {error}"))?;
    writer
        .finish()
        .map_err(|error| eyre!("Couldn't encode the screenshot: {error}"))?;
    Ok(bytes)
}

/// `text` cut to [`MAX_SELECTION`] characters.
fn clip_selection(text: String) -> String {
    match text.char_indices().nth(MAX_SELECTION) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::assistant::{Context, TaskWindow};

    pub fn screenshot() -> Result<Screenshot> {
        use scrawl_capture::screens;
        let at = screens::pointer().unwrap_or_default();
        let displays = screens::displays();
        let display = screens::index_at(&displays, at)
            .map(|index| displays[index])
            .ok_or_else(|| eyre!("No display to take a screenshot of"))?;
        let image = capture_core::screenshot_with(
            &capture_core::Source::Display(display.id),
            &exclusions(),
            Some(LONG_SIDE),
            true,
        )
        .map_err(|error| {
            if permissions(false).screen_recording {
                eyre!("Couldn't take a screenshot: {error}")
            } else {
                eyre!("Hex needs Screen Recording to see the screen")
            }
        })?;
        let (width, height) = fit(image.width, image.height, LONG_SIDE);
        let rgba = resize(&image.rgba, (image.width, image.height), (width, height));
        Ok(Screenshot {
            png: encode_png(width, height, &rgba)?,
            width,
            height,
        })
    }

    /// Hex's own overlays (the HUD, Agent mode's frame and outlines): never
    /// part of what the model sees. Hex's ordinary windows (Chats) stay.
    fn exclusions() -> capture_core::Exclusions {
        let pid = std::process::id() as i32;
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows())
            .into_iter()
            .filter(|window| window.pid == pid && window.layer != 0)
            .map(|window| window.id)
            .collect();
        capture_core::Exclusions {
            windows,
            apps: Vec::new(),
        }
    }

    pub fn window_frame(window: &TaskWindow) -> Option<(f64, f64, f64, f64)> {
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
        let shown = |candidate: &&scrawl_capture::windows::RawWindow| {
            candidate.alpha > 0.0 && candidate.bounds[2] > 1.0 && candidate.bounds[3] > 1.0
        };
        let found = match window.window_id.as_deref().and_then(window_number) {
            Some(number) => windows
                .iter()
                .filter(shown)
                .find(|candidate| candidate.id == number),
            // The app's frontmost ordinary window (the list is front to back).
            None => windows.iter().filter(shown).find(|candidate| {
                candidate.layer == 0 && candidate.owner.eq_ignore_ascii_case(window.app.trim())
            }),
        }?;
        let [x, y, width, height] = found.bounds;
        Some((x, y, width, height))
    }

    pub fn context(with_screenshot: bool) -> Context {
        let front = objc2::rc::autoreleasepool(|_| scrawl_capture::capture::frontmost_app_info());
        let (selected_text, window_title) = match &front {
            Some((pid, _, _)) => (
                crate::accessibility::selected_text(*pid).map(clip_selection),
                crate::accessibility::focused_window_title(*pid),
            ),
            None => (None, None),
        };
        let screenshot = with_screenshot
            .then(|| {
                super::screenshot()
                    .inspect_err(|error| {
                        tracing::warn!(error = %format!("{error:#}"), "no screenshot for the question");
                    })
                    .ok()
            })
            .flatten();
        Context {
            screenshot,
            selected_text,
            frontmost_app: front.map(|(_, name, _)| name),
            window_title,
        }
    }

    pub fn permissions(fresh: bool) -> Permissions {
        use crate::capture_host::ScreenAccess;
        use crate::desktop::onboarding::PermissionState;
        let accessibility = crate::desktop::onboarding::status_with_transcription_model(true)
            .accessibility
            == PermissionState::Ready;
        let screen = if fresh {
            crate::capture_host::screen_access_now()
        } else {
            crate::capture_host::screen_access()
        };
        Permissions {
            accessibility,
            screen_recording: screen == ScreenAccess::Allowed,
        }
    }

    pub fn open_settings(permission: Permission) {
        use crate::desktop::onboarding::PermissionKind;
        let kind = match permission {
            Permission::Accessibility => PermissionKind::Accessibility,
            Permission::ScreenRecording => PermissionKind::ScreenRecording,
        };
        if objc2::MainThreadMarker::new().is_some() {
            crate::desktop::permission_guide::show(kind);
            return;
        }
        let pane = match permission {
            Permission::Accessibility => "Privacy_Accessibility",
            Permission::ScreenRecording => "Privacy_ScreenCapture",
        };
        let url = format!(
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?{pane}"
        );
        if let Err(error) = crate::desktop::background_process::command("/usr/bin/open")
            .arg(url)
            .spawn()
            .and_then(|mut child| child.wait())
        {
            tracing::warn!(%error, ?permission, "could not open System Settings");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_screen_is_fitted_to_the_long_side() {
        assert_eq!(fit(3600, 2338, LONG_SIDE), (1280, 831));
        assert_eq!(fit(2338, 3600, LONG_SIDE), (831, 1280));
        assert_eq!(fit(5120, 2880, LONG_SIDE), (1280, 720));
        assert_eq!(fit(1280, 800, LONG_SIDE), (1280, 800));
        assert_eq!(fit(800, 600, LONG_SIDE), (800, 600));
        assert_eq!(fit(100_000, 10, LONG_SIDE), (1280, 1));
    }

    #[test]
    fn resizing_averages_the_pixels_each_target_pixel_covers() {
        // 4×2: a black and a white 2×2 block, side by side.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend([0, 0, 0, 255].repeat(2));
            rgba.extend([255, 255, 255, 255].repeat(2));
        }
        assert_eq!(
            resize(&rgba, (4, 2), (2, 1)),
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
        assert_eq!(resize(&rgba, (4, 2), (1, 1)), [127, 127, 127, 255]);
        assert_eq!(resize(&rgba, (4, 2), (4, 2)), rgba);
        // A ratio that doesn't divide evenly still fills every pixel.
        assert_eq!(resize(&[9; 3 * 3 * 4], (3, 3), (2, 2)).len(), 2 * 2 * 4);
    }

    #[test]
    fn a_resized_screenshot_encodes_as_png() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (2, 1));
    }

    #[test]
    fn agent_desktop_window_ids_carry_the_cg_window_number() {
        assert_eq!(window_number("w-14569"), Some(14569));
        assert_eq!(window_number(" w-7 "), Some(7));
        assert_eq!(window_number("14569"), None);
        assert_eq!(window_number("w-x"), None);
    }

    #[test]
    fn a_long_selection_is_clipped() {
        let long = "a".repeat(MAX_SELECTION + 5);
        assert_eq!(clip_selection(long).chars().count(), MAX_SELECTION + 1);
        assert_eq!(clip_selection("short".into()), "short");
    }
}
