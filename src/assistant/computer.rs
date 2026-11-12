//! What Hex itself reads from the desktop: a quick question's context
//! (screenshot, selected text, frontmost app) and where a task's windows
//! are, for their outlines. Tasks operate apps through agent-desktop
//! (`backends`), not through this module.
//!
//! Everything here only reads: the selection comes through the OS's
//! accessibility API, never the clipboard, and the screenshot never shows
//! Hex's own windows (the HUD, the frame, Chats), blocked apps' windows, or
//! password fields.
//!
//! - macOS: Accessibility, and ScreenCaptureKit with those windows left out.
//! - Windows: UI Automation, and GDI with what shows of them blacked out
//!   ([`windows`]); coordinates are global physical pixels.
//! - Linux: AT-SPI; on X11 the root window with what shows of them blacked
//!   out, on Wayland the Screenshot portal ([`linux`]).
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

mod displays;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(test, not(target_os = "macos")))]
mod mask;
mod privacy;
#[cfg(target_os = "windows")]
mod windows;

pub use displays::{
    Display, agents_display, cascade, describe as describe_displays, display_of, displays,
};

/// `assistant.agent_display` as last applied.
pub fn agent_display_setting() -> Option<String> {
    crate::app_settings::assistant_runtime().and_then(|(_, settings)| settings.agent_display)
}

/// A screenshot of the display the person works on (the one holding the
/// focused window, else the pointer's), scaled so its long side is at most
/// 1280 pixels.
pub fn screenshot() -> Result<Screenshot> {
    #[cfg(target_os = "macos")]
    {
        macos::screenshot()
    }
    #[cfg(target_os = "windows")]
    {
        windows::screenshot()
    }
    #[cfg(target_os = "linux")]
    {
        linux::screenshot()
    }
}

/// Where `window` is on screen, in global display points (x, y, width,
/// height), for its task's outline; `None` when it's gone or hidden.
///
/// A window id is agent-desktop's: `w-4521` on macOS (the CoreGraphics
/// window number) and Windows (`w-<hwnd>`), the window's accessible address
/// on Linux; without one, the app's frontmost window. Windows gives global
/// pixels; Wayland tells no window's place (`None`).
pub fn window_frame(window: &crate::assistant::TaskWindow) -> Option<(f64, f64, f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        macos::window_frame(window)
    }
    #[cfg(target_os = "windows")]
    {
        windows::window_frame(window)
    }
    #[cfg(target_os = "linux")]
    {
        linux::window_frame(window)
    }
}

/// The Accessibility part of [`context`] alone (the frontmost app, its
/// focused window's title, the selected text): no screenshot, a few
/// milliseconds, each read bounded at a quarter second.
pub fn quick_context() -> crate::assistant::Context {
    #[cfg(target_os = "macos")]
    {
        macos::quick_context()
    }
    #[cfg(target_os = "windows")]
    {
        windows::quick_context()
    }
    #[cfg(target_os = "linux")]
    {
        linux::quick_context()
    }
}

/// Starts reading the desktop on its own thread and returns at once: the
/// Accessibility part first, then (when `with_screenshot`) the screenshot.
/// Dropping the handle cancels it: a read not yet at its screenshot takes
/// none, and what a finished read holds is freed, never kept.
pub fn context_async(with_screenshot: bool) -> ContextHandle {
    ContextHandle::start(move |read| {
        read.quick(quick_context());
        if with_screenshot && !read.cancelled() {
            match screenshot() {
                Ok(shot) => read.screenshot(shot),
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "no screenshot for the question");
                }
            }
        }
    })
}

/// A desktop read under way ([`context_async`]).
pub struct ContextHandle {
    shared: std::sync::Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    state: std::sync::Mutex<Read>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct Read {
    context: Option<crate::assistant::Context>,
    done: bool,
    cancelled: bool,
}

/// The reading thread's side of a [`ContextHandle`].
pub struct Reading {
    shared: std::sync::Arc<Shared>,
}

impl Reading {
    fn update(&self, change: impl FnOnce(&mut Read)) {
        let mut read = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !read.cancelled {
            change(&mut read);
        }
        self.shared.changed.notify_all();
    }

    /// The Accessibility part arrived.
    pub fn quick(&self, context: crate::assistant::Context) {
        self.update(|read| read.context = Some(context));
    }

    /// The screenshot arrived (dropped at once if the handle is gone).
    pub fn screenshot(&self, shot: Screenshot) {
        self.update(|read| {
            read.context.get_or_insert_with(Default::default).screenshot = Some(shot);
        });
    }

    pub fn cancelled(&self) -> bool {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        let mut read = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        read.done = true;
        self.shared.changed.notify_all();
    }
}

impl ContextHandle {
    /// Runs `read` on its own thread; the seam tests drive.
    pub fn start(read: impl FnOnce(&Reading) + Send + 'static) -> Self {
        let shared = std::sync::Arc::new(Shared::default());
        let reading = Reading {
            shared: shared.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-context".into())
            .spawn(move || read(&reading));
        if let Err(error) = spawned {
            // `reading` dropped with the closure: the handle reads as done.
            tracing::warn!(%error, "could not read the desktop for a question");
        }
        Self { shared }
    }

    /// Whether the read has finished (or failed to start).
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .done
    }

    /// The context, waiting at most `timeout` for the read to finish; past
    /// it, whatever has arrived (the Accessibility part, usually), and the
    /// rest is dropped when it comes.
    pub fn wait(self, timeout: std::time::Duration) -> crate::assistant::Context {
        let deadline = std::time::Instant::now() + timeout;
        let mut read = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while !read.done {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            read = self
                .shared
                .changed
                .wait_timeout(read, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        read.context.take().unwrap_or_default()
    }
}

impl Drop for ContextHandle {
    fn drop(&mut self) {
        let mut read = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        read.cancelled = true;
        // A screenshot nobody will send is freed now, not when the thread ends.
        read.context = None;
    }
}

/// Whether Hex may read other apps and take screenshots, for setup to show
/// what's missing, read afresh so a grant made since launch shows; blocks
/// for up to a few seconds. Off the UI thread.
///
/// - macOS: Accessibility and Screen Recording.
/// - Windows: nothing to grant; both are true.
/// - Linux: `accessibility` is the accessibility bus being on (AT-SPI);
///   `screen_recording` is true on X11, and on Wayland once the Screenshot
///   portal allowed Hex.
pub fn permissions_now() -> Permissions {
    #[cfg(target_os = "macos")]
    {
        macos::permissions()
    }
    #[cfg(target_os = "windows")]
    {
        windows::permissions()
    }
    #[cfg(target_os = "linux")]
    {
        linux::permissions()
    }
}

/// What Hex may do, each true where nothing needs granting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Permissions {
    /// Read other apps' text: [`Permission::Accessibility`].
    pub accessibility: bool,
    /// Take screenshots: [`Permission::ScreenRecording`].
    pub screen_recording: bool,
}

/// One permission Hex asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Permission {
    /// macOS's Accessibility; Linux's accessibility bus.
    Accessibility,
    /// macOS's Screen Recording; Wayland's Screenshot portal.
    ScreenRecording,
}

impl Permission {
    /// Its name where it is granted (System Settings on macOS), in English
    /// for `tr()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Accessibility if cfg!(target_os = "linux") => "Accessibility bus",
            Self::ScreenRecording if cfg!(target_os = "linux") => "Screenshots",
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "Screen Recording",
        }
    }
}

/// Gets `permission` granted.
///
/// - macOS: opens System Settings at its list. On the main thread Hex's
///   own guide floats beside it with Hex to drag in (as onboarding does);
///   elsewhere the list opens alone.
/// - Linux: turns the accessibility bus on at once; on Wayland, asks the
///   Screenshot portal, which shows its question.
/// - Windows: nothing to grant.
pub fn open_settings(permission: Permission) {
    #[cfg(target_os = "macos")]
    macos::open_settings(permission);
    #[cfg(target_os = "linux")]
    linux::open_settings(permission);
    #[cfg(target_os = "windows")]
    let _ = permission;
}

/// The number in an agent-desktop window id (`w-4521`): the CoreGraphics
/// window number on macOS, the window handle on Windows.
pub fn window_number(id: &str) -> Option<u32> {
    id.trim().strip_prefix("w-")?.parse().ok()
}

/// The bus name and object path an agent-desktop window id on Linux is made
/// of (`:1.42/org/a11y/atspi/accessible/7`).
#[cfg(any(test, target_os = "linux"))]
pub fn atspi_address(id: &str) -> Option<(String, String)> {
    let id = id.trim();
    let slash = id.find('/')?;
    let (name, path) = id.split_at(slash);
    let name_ok = name.len() > 1
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'));
    let path_ok = path.len() > 1
        && path.split('/').skip(1).all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    (name_ok && path_ok).then(|| (name.to_owned(), path.to_owned()))
}

/// What a portal permission store's table says for `app_id`: `yes` or
/// `no`, `None` when it says nothing.
#[cfg(any(test, target_os = "linux"))]
pub fn stored_answer(
    permissions: &std::collections::HashMap<String, Vec<String>>,
    app_id: &str,
) -> Option<bool> {
    match permissions.get(app_id)?.first()?.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// Where `display` lies in a picture of the whole desktop that is `size`
/// pixels (the bounding box of `displays`, at one scale): `[x, y, width,
/// height]` in its pixels, or `None` when the picture isn't that layout.
#[cfg(any(test, target_os = "linux"))]
pub fn crop_rect(
    displays: &[Display],
    display: &Display,
    (width, height): (u32, u32),
) -> Option<[u32; 4]> {
    let left = displays
        .iter()
        .map(|d| d.frame.0)
        .fold(f64::INFINITY, f64::min);
    let top = displays
        .iter()
        .map(|d| d.frame.1)
        .fold(f64::INFINITY, f64::min);
    let right = displays
        .iter()
        .map(|d| d.frame.0 + d.frame.2)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = displays
        .iter()
        .map(|d| d.frame.1 + d.frame.3)
        .fold(f64::NEG_INFINITY, f64::max);
    if displays.len() < 2 || right <= left || bottom <= top {
        return None;
    }
    let scale = f64::from(width) / (right - left);
    // The picture must have the layout's shape, or it's not this layout.
    if ((bottom - top) * scale - f64::from(height)).abs() > 2.0 {
        return None;
    }
    let (x, y, w, h) = display.frame;
    let rect = [
        ((x - left) * scale).round() as u32,
        ((y - top) * scale).round() as u32,
        (w * scale).round() as u32,
        (h * scale).round() as u32,
    ];
    (rect[0] + rect[2] <= width && rect[1] + rect[3] <= height && rect[2] > 0 && rect[3] > 0)
        .then_some(rect)
}

/// The `[x, y, width, height]` part of a tightly packed RGBA image `width`
/// pixels wide.
#[cfg(any(test, target_os = "linux"))]
pub fn crop(rgba: &[u8], width: u32, [x, y, w, h]: [u32; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * width as usize + x as usize) * 4;
        out.extend_from_slice(&rgba[start..start + w as usize * 4]);
    }
    out
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

/// `window`'s frame among CoreGraphics' on-screen windows (front to back):
/// their bounds as they come, global points with the origin at the primary
/// display's top-left corner and y growing down (a display above the
/// primary one has negative y), which is what the overlays draw in.
#[cfg(any(test, not(target_os = "linux")))]
pub fn frame_among(
    windows: &[scrawl_capture::windows::RawWindow],
    window: &crate::assistant::TaskWindow,
) -> Option<(f64, f64, f64, f64)> {
    let shown = |candidate: &&scrawl_capture::windows::RawWindow| {
        candidate.alpha > 0.0 && candidate.bounds[2] > 1.0 && candidate.bounds[3] > 1.0
    };
    let found = match window.window_id.as_deref().and_then(window_number) {
        Some(number) => windows
            .iter()
            .filter(shown)
            .find(|candidate| candidate.id == number),
        // The app's frontmost ordinary window.
        None => windows.iter().filter(shown).find(|candidate| {
            candidate.layer == 0 && candidate.owner.eq_ignore_ascii_case(window.app.trim())
        }),
    }?;
    let [x, y, width, height] = found.bounds;
    Some((x, y, width, height))
}

/// App `pid`'s window in front, as agent-desktop names it (`w-<number>`):
/// the Here lane's window. That is the window the app says has focus
/// (`focused`, through Accessibility) when CoreGraphics shows it, else the
/// app's frontmost ordinary window among CoreGraphics' on-screen windows
/// (front to back).
#[cfg(any(test, not(target_os = "linux")))]
pub fn front_window_among(
    windows: &[scrawl_capture::windows::RawWindow],
    pid: i32,
    app: &str,
    focused: Option<u32>,
) -> Option<crate::assistant::TaskWindow> {
    let said = focused.filter(|number| {
        windows
            .iter()
            .any(|candidate| candidate.id == *number && candidate.pid == pid)
    });
    let number = match said {
        Some(number) => number,
        None => {
            windows
                .iter()
                .find(|candidate| {
                    candidate.pid == pid
                        && candidate.layer == 0
                        && candidate.alpha > 0.0
                        && candidate.bounds[2] > 1.0
                        && candidate.bounds[3] > 1.0
                })?
                .id
        }
    };
    Some(crate::assistant::TaskWindow {
        app: app.to_owned(),
        window_id: Some(format!("w-{number}")),
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::assistant::{Context, TaskWindow};

    pub fn screenshot() -> Result<Screenshot> {
        use scrawl_capture::screens;
        // The display the person works on: the focused window's, else the
        // pointer's.
        let focused = super::displays()
            .into_iter()
            .find(|display| display.has_focus)
            .and_then(|display| {
                let (x, y, w, h) = display.frame;
                Some((display.id.parse::<u32>().ok()?, [x, y, w, h]))
            });
        let (id, frame) = match focused {
            Some(found) => found,
            None => {
                let at = screens::pointer().unwrap_or_default();
                let displays = screens::displays();
                screens::index_at(&displays, at)
                    .map(|index| (displays[index].id, displays[index].frame))
                    .ok_or_else(|| eyre!("No display to take a screenshot of"))?
            }
        };
        // Password fields, found before the shot (they don't move in it).
        let secure = super::privacy::secure_fields_on(frame);
        let mut image = capture_core::screenshot_with(
            &capture_core::Source::Display(id),
            &exclusions(),
            Some(LONG_SIDE),
            true,
        )
        .map_err(|error| {
            if crate::capture_host::screen_access() == crate::capture_host::ScreenAccess::Allowed {
                eyre!("Couldn't take a screenshot: {error}")
            } else {
                eyre!("Hex needs Screen Recording to see the screen")
            }
        })?;
        super::privacy::black_out(&mut image.rgba, (image.width, image.height), frame, &secure);
        let (width, height) = fit(image.width, image.height, LONG_SIDE);
        let rgba = resize(&image.rgba, (image.width, image.height), (width, height));
        Ok(Screenshot {
            png: encode_png(width, height, &rgba)?,
            width,
            height,
        })
    }

    /// Every window of Hex's (the HUD, Agent mode's frame and outlines,
    /// Chats) and of the apps the person blocked: never part of what the
    /// model sees. The filter leaves out whole apps, so a window one shows
    /// while the shot is taken is left out too.
    fn exclusions() -> capture_core::Exclusions {
        let mut apps = vec![std::process::id() as i32];
        apps.extend(super::privacy::blocked_pids_now());
        capture_core::Exclusions {
            windows: Vec::new(),
            apps,
        }
    }

    pub fn window_frame(window: &TaskWindow) -> Option<(f64, f64, f64, f64)> {
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
        super::frame_among(&windows, window)
    }

    pub fn quick_context() -> Context {
        let front = objc2::rc::autoreleasepool(|_| scrawl_capture::capture::frontmost_app_info());
        let (selected_text, window_title) = match &front {
            Some((pid, _, _)) => (
                crate::accessibility::selected_text(*pid).map(clip_selection),
                crate::accessibility::focused_window_title(*pid),
            ),
            None => (None, None),
        };
        let window = front.as_ref().and_then(|(pid, name, _)| {
            let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
            let focused = crate::accessibility::focused_window_number(*pid);
            super::front_window_among(&windows, *pid, name, focused)
        });
        let displays = super::displays();
        let chosen = super::agent_display_setting();
        let agents = super::agents_display(&displays, chosen.as_deref());
        Context {
            screenshot: None,
            selected_text,
            frontmost_app: front.map(|(_, name, _)| name),
            window_title,
            window,
            display: super::displays::describe(&displays, agents),
        }
    }

    pub fn permissions() -> Permissions {
        use crate::capture_host::ScreenAccess;
        use crate::desktop::onboarding::PermissionState;
        let accessibility = crate::desktop::onboarding::status_with_transcription_model(true)
            .accessibility
            == PermissionState::Ready;
        let screen = crate::capture_host::screen_access_now();
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
    fn linux_window_ids_are_accessible_addresses() {
        assert_eq!(
            atspi_address(":1.42/org/a11y/atspi/accessible/7"),
            Some((":1.42".into(), "/org/a11y/atspi/accessible/7".into()))
        );
        assert_eq!(
            atspi_address("org.gnome.TextEditor/org/gtk/Application/anonymous/a11y/12"),
            Some((
                "org.gnome.TextEditor".into(),
                "/org/gtk/Application/anonymous/a11y/12".into()
            ))
        );
        assert_eq!(atspi_address("w-14569"), None);
        assert_eq!(atspi_address(":1.42/"), None);
        assert_eq!(atspi_address("/org/a11y"), None);
    }

    #[test]
    fn the_portal_store_says_yes_no_or_nothing() {
        let store = std::collections::HashMap::from([
            ("anomaly.hex".to_owned(), vec!["yes".to_owned()]),
            ("other".to_owned(), vec!["no".to_owned()]),
        ]);
        assert_eq!(stored_answer(&store, "anomaly.hex"), Some(true));
        assert_eq!(stored_answer(&store, "other"), Some(false));
        assert_eq!(stored_answer(&store, "missing"), None);
    }

    #[test]
    fn a_desktop_picture_is_cut_to_the_persons_display() {
        let display = |id: &str, frame| Display {
            id: id.into(),
            name: id.into(),
            frame,
            visible_frame: frame,
            scale: 1.0,
            is_primary: false,
            has_focus: false,
        };
        let displays = [
            display("1", (0.0, 0.0, 1920.0, 1080.0)),
            display("2", (1920.0, -200.0, 1280.0, 1024.0)),
        ];
        // 3200 × 1280 points, pictured at 2×.
        assert_eq!(
            crop_rect(&displays, &displays[1], (6400, 2560)),
            Some([3840, 0, 2560, 2048])
        );
        assert_eq!(
            crop_rect(&displays, &displays[0], (6400, 2560)),
            Some([0, 400, 3840, 2160])
        );
        // A picture of another shape, or one display: the whole picture.
        assert_eq!(crop_rect(&displays, &displays[0], (6400, 1000)), None);
        assert_eq!(crop_rect(&displays[..1], &displays[0], (1920, 1080)), None);
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        assert_eq!(crop(&rgba, 4, [1, 1, 2, 1]), rgba[20..28]);
    }

    #[test]
    fn a_long_selection_is_clipped() {
        let long = "a".repeat(MAX_SELECTION + 5);
        assert_eq!(clip_selection(long).chars().count(), MAX_SELECTION + 1);
        assert_eq!(clip_selection("short".into()), "short");
    }

    fn raw(
        id: u32,
        owner: &str,
        bounds: [f64; 4],
        layer: i32,
    ) -> scrawl_capture::windows::RawWindow {
        scrawl_capture::windows::RawWindow {
            id,
            pid: 1,
            owner: owner.into(),
            title: String::new(),
            bounds,
            layer,
            alpha: 1.0,
        }
    }

    fn task(app: &str, id: Option<&str>) -> crate::assistant::TaskWindow {
        crate::assistant::TaskWindow {
            app: app.into(),
            window_id: id.map(str::to_owned),
        }
    }

    /// The overlays draw in global points, top-left origin at the primary
    /// display, y down: a frame comes back as CoreGraphics lists it, so a
    /// window on a display above the primary one has negative y, and one at
    /// the primary display's top has y 0 (never flipped to AppKit's y up).
    #[test]
    fn window_frames_are_global_top_left_points_with_y_down() {
        let windows = [
            raw(24, "Window Server", [0.0, 0.0, 1800.0, 24.0], 24),
            raw(14569, "Citrix Viewer", [-604.0, -1440.0, 2560.0, 1440.0], 0),
            raw(7, "Mail", [100.0, 39.0, 900.0, 700.0], 0),
            raw(8, "Mail", [300.0, 200.0, 900.0, 700.0], 0),
        ];
        assert_eq!(
            frame_among(&windows, &task("Citrix Viewer", Some("w-14569"))),
            Some((-604.0, -1440.0, 2560.0, 1440.0))
        );
        // Without an id, the app's frontmost (first listed) ordinary window.
        assert_eq!(
            frame_among(&windows, &task("mail", None)),
            Some((100.0, 39.0, 900.0, 700.0))
        );
        assert_eq!(frame_among(&windows, &task("Mail", Some("w-99"))), None);
        let mut hidden = raw(9, "Notes", [0.0, 0.0, 400.0, 300.0], 0);
        hidden.alpha = 0.0;
        assert_eq!(frame_among(&[hidden], &task("Notes", None)), None);
    }

    #[test]
    fn the_window_in_front_is_named_as_agent_desktop_names_it() {
        let mut other = raw(5, "Notes", [0.0, 39.0, 400.0, 300.0], 0);
        other.pid = 2;
        let windows = [
            raw(24, "Mail", [0.0, 0.0, 1800.0, 24.0], 24),
            other,
            raw(7, "Mail", [100.0, 39.0, 900.0, 700.0], 0),
            raw(8, "Mail", [300.0, 200.0, 900.0, 700.0], 0),
        ];
        assert_eq!(
            front_window_among(&windows, 1, "Mail", None),
            Some(task("Mail", Some("w-7")))
        );
        // The window Mail says has focus, behind a strip of its own.
        assert_eq!(
            front_window_among(&windows, 1, "Mail", Some(8)),
            Some(task("Mail", Some("w-8")))
        );
        // Not another app's window.
        assert_eq!(
            front_window_among(&windows, 1, "Mail", Some(5)),
            Some(task("Mail", Some("w-7")))
        );
        assert_eq!(front_window_among(&windows, 3, "Gone", None), None);
    }

    /// Live, read-only: the menu bar sits at the top of the primary display,
    /// so in these coordinates its y is 0 (y up would put it near the
    /// display's height). Skipped where no menu bar is on screen.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_menu_bar_is_at_y_zero_on_this_mac() {
        let windows = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
        let Some(menu_bar) = windows
            .iter()
            .find(|window| window.title == "Menubar" && window.bounds[0] == 0.0)
        else {
            return;
        };
        assert_eq!(menu_bar.bounds[1], 0.0, "{menu_bar:?}");
        let found = frame_among(
            &windows,
            &task(&menu_bar.owner, Some(&format!("w-{}", menu_bar.id))),
        );
        assert_eq!(found.map(|frame| frame.1), Some(0.0));
    }

    fn context_named(app: &str) -> crate::assistant::Context {
        crate::assistant::Context {
            frontmost_app: Some(app.into()),
            ..Default::default()
        }
    }

    fn shot() -> Screenshot {
        Screenshot {
            png: vec![1; 1024],
            width: 1,
            height: 1,
        }
    }

    #[test]
    fn a_finished_read_hands_over_everything() {
        let handle = ContextHandle::start(|read| {
            read.quick(context_named("Mail"));
            read.screenshot(shot());
        });
        let context = handle.wait(std::time::Duration::from_secs(5));
        assert_eq!(context.frontmost_app.as_deref(), Some("Mail"));
        assert_eq!(context.screenshot, Some(shot()));
    }

    /// Past the wait, the quick part goes alone and a slow screenshot is
    /// dropped when it arrives.
    #[test]
    fn a_slow_screenshot_is_left_behind() {
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (finished, finish) = std::sync::mpsc::channel::<bool>();
        let handle = ContextHandle::start(move |read| {
            read.quick(context_named("Safari"));
            let _ = released.recv();
            read.screenshot(shot());
            let _ = finished.send(read.cancelled());
        });
        let context = handle.wait(std::time::Duration::from_millis(100));
        assert_eq!(context.frontmost_app.as_deref(), Some("Safari"));
        assert_eq!(context.screenshot, None);
        release.send(()).unwrap();
        assert_eq!(
            finish.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(true)
        );
    }

    /// Dropping the handle (a ⌘C chord, a discarded hold) cancels: a read
    /// not yet at its screenshot skips it, and nothing is kept.
    #[test]
    fn a_dropped_read_takes_no_screenshot_and_keeps_nothing() {
        let (go, gone) = std::sync::mpsc::channel::<()>();
        let (took, taken) = std::sync::mpsc::channel::<bool>();
        let handle = ContextHandle::start(move |read| {
            let _ = gone.recv();
            read.quick(context_named("Notes"));
            let shoot = !read.cancelled();
            if shoot {
                read.screenshot(shot());
            }
            let _ = took.send(shoot);
        });
        let shared = handle.shared.clone();
        drop(handle);
        go.send(()).unwrap();
        assert_eq!(
            taken.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(false)
        );
        let read = shared.state.lock().unwrap();
        assert!(read.cancelled);
        assert!(read.context.is_none());
    }

    #[test]
    fn a_read_that_ends_reports_done() {
        let handle = ContextHandle::start(|read| read.quick(context_named("Finder")));
        let started = std::time::Instant::now();
        while !handle.is_done() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(handle.is_done());
        assert_eq!(
            handle
                .wait(std::time::Duration::ZERO)
                .frontmost_app
                .as_deref(),
            Some("Finder")
        );
    }
}
