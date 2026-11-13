//! What Agent-Desk reads from the Windows desktop for a question: the window in
//! front (its app and title), the selected text (UI Automation's text
//! pattern on the focused element, bounded), the displays, the windows'
//! frames, and a screenshot of the display the person works on (GDI, at
//! the display's own pixels; Agent-Desk's overlays exclude themselves from
//! capture), with blocked apps' windows blacked out.

use std::sync::mpsc;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use scrawl_capture::win;

use super::Screenshot;
use super::displays::{Display, Frame};
use crate::assistant::{Context, TaskWindow};

/// How long the focused app may take to say what's selected.
const SELECTION_WAIT: Duration = Duration::from_millis(250);

/// The displays, the primary first, with the one the person works on
/// marked (the foreground window's, else the pointer's).
pub fn displays() -> Vec<Display> {
    let mut displays: Vec<Display> = win::monitors()
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let [x, y, width, height] = monitor.rect;
            let [vx, vy, vw, vh] = monitor.work;
            Display {
                id: (index + 1).to_string(),
                name: if monitor.primary {
                    "Main display".to_owned()
                } else {
                    format!("Display {}", index + 1)
                },
                frame: (x, y, width, height),
                visible_frame: (vx, vy, vw, vh),
                scale: monitor.scale,
                is_primary: monitor.primary,
                has_focus: false,
            }
        })
        .collect();
    let focused = win::foreground_window().map(|window| {
        let [x, y, width, height] = window.rect;
        (x, y, width, height)
    });
    super::displays::mark_focus(&mut displays, focused, win::cursor_position());
    displays
}

/// The app names (and executable ids, `keepassxc.exe`) of the windows on
/// screen, with their frames.
fn windows_with_apps() -> Vec<(String, String, [f64; 4])> {
    win::top_level_windows()
        .into_iter()
        .filter(|window| window.on_screen())
        .filter_map(|window| {
            let (name, id) = win::app_identity(window.pid)?;
            Some((name, id, window.rect))
        })
        .collect()
}

/// Whether an app named `name` (executable `id`) is one of `blocked`.
fn blocked(name: &str, id: &str, blocked: &[String]) -> bool {
    let stem = id.strip_suffix(".exe").unwrap_or(id);
    blocked.iter().any(|app| {
        let app = app.trim();
        !app.is_empty()
            && (name.eq_ignore_ascii_case(app)
                || id.eq_ignore_ascii_case(app)
                || stem.eq_ignore_ascii_case(app))
    })
}

/// A screenshot of the display the person works on.
pub fn screenshot() -> Result<Screenshot> {
    let display = displays()
        .into_iter()
        .find(|display| display.has_focus)
        .ok_or_else(|| eyre!("No display to take a screenshot of"))?;
    let (x, y, width, height) = display.frame;
    let (pixel_width, pixel_height) = (width.round() as u32, height.round() as u32);
    let mut rgba = grab(
        x.round() as i32,
        y.round() as i32,
        pixel_width,
        pixel_height,
    )?;
    let blocked_apps = super::privacy::blocked_apps();
    let hidden: Vec<[f64; 4]> = windows_with_apps()
        .into_iter()
        .filter(|(name, id, _)| blocked(name, id, &blocked_apps))
        .map(|(_, _, frame)| frame)
        .collect();
    super::privacy::black_out(
        &mut rgba,
        (pixel_width, pixel_height),
        [x, y, width, height],
        &hidden,
    );
    super::screenshot_from(pixel_width, pixel_height, &rgba)
}

/// The screen's pixels in the rectangle at (`x`, `y`), `width` × `height`
/// physical pixels, as RGBA.
fn grab(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    if width == 0 || height == 0 {
        return Err(eyre!("The display has no pixels"));
    }
    let (w, h) = (
        i32::try_from(width).map_err(|_| eyre!("The display is too wide"))?,
        i32::try_from(height).map_err(|_| eyre!("The display is too tall"))?,
    );
    // SAFETY: plain GDI calls on handles made here and released below; the
    // buffer holds `width * height` 32-bit pixels for GetDIBits.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err(eyre!("Couldn't reach the screen"));
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(memory, bitmap);
        let copied = BitBlt(memory, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: rows top to bottom.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bgra = vec![0_u8; width as usize * height as usize * 4];
        let rows = if copied != 0 {
            GetDIBits(
                memory,
                bitmap,
                0,
                height,
                bgra.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
        if rows == 0 {
            return Err(eyre!("Couldn't take a screenshot"));
        }
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(bgra)
    }
}

/// The text selected in the focused element, through UI Automation, on its
/// own thread so a hung app's provider costs at most [`SELECTION_WAIT`].
fn selected_text() -> Option<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("hex-assistant-selection".into())
        .spawn(move || {
            let _ = sender.send(read_selection());
        })
        .ok()?;
    receiver
        .recv_timeout(SELECTION_WAIT)
        .ok()
        .flatten()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(super::clip_selection)
}

fn read_selection() -> Option<String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
    };
    // SAFETY: COM is joined and left on this thread; every interface is a
    // smart pointer released before CoUninitialize.
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED);
        let text = (|| -> windows::core::Result<Option<String>> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let focused = automation.GetFocusedElement()?;
            let Ok(pattern) =
                focused.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            else {
                return Ok(None);
            };
            let ranges = pattern.GetSelection()?;
            let mut parts = Vec::new();
            for index in 0..ranges.Length()? {
                let text = ranges.GetElement(index)?.GetText(-1)?.to_string();
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            Ok((!parts.is_empty()).then(|| parts.join("\n")))
        })();
        if com.is_ok() {
            CoUninitialize();
        }
        text.ok().flatten()
    }
}

/// The window in front, the selected text, and the displays.
pub fn quick_context() -> Context {
    let front = win::foreground_window();
    let app = front
        .as_ref()
        .and_then(|window| win::app_identity(window.pid))
        .map(|(name, _)| name);
    let displays = super::displays();
    let chosen = super::agent_display_setting();
    let agents = super::agents_display(&displays, chosen.as_deref());
    Context {
        screenshot: None,
        selected_text: selected_text(),
        window: app.as_ref().map(|app| TaskWindow {
            app: app.clone(),
            window_id: front.as_ref().map(|window| format!("w-{}", window.hwnd)),
        }),
        frontmost_app: app,
        window_title: front
            .map(|window| window.title)
            .filter(|title| !title.is_empty()),
        display: super::displays::describe(&displays, agents),
    }
}

/// `window`'s frame: the window its id names (`w-<handle>`), else
/// its app's frontmost window.
pub fn window_frame(window: &TaskWindow) -> Option<Frame> {
    let by_id = window
        .window_id
        .as_deref()
        .and_then(|id| id.trim().strip_prefix("w-"))
        .and_then(|id| id.parse::<isize>().ok())
        .and_then(win::window_info)
        .filter(|found| found.on_screen());
    let found = by_id.or_else(|| {
        win::top_level_windows().into_iter().find(|candidate| {
            candidate.on_screen()
                && candidate.is_app_window()
                && win::app_identity(candidate.pid).is_some_and(|(name, id)| {
                    blocked(&name, &id, std::slice::from_ref(&window.app))
                })
        })
    })?;
    let [x, y, width, height] = found.rect;
    Some((x, y, width, height))
}

/// Every running app's executable (one per window on screen).
pub fn process_paths() -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = win::top_level_windows()
        .into_iter()
        .filter_map(|window| win::process_path(window.pid))
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_app_is_known_by_name_or_executable() {
        assert!(blocked("KeePassXC", "keepassxc.exe", &["keepassxc".into()]));
        assert!(blocked("1Password", "1password.exe", &["1Password".into()]));
        assert!(!blocked("Notepad", "notepad.exe", &["Bitwarden".into()]));
    }
}
