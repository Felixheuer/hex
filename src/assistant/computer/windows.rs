//! Hex's own reads of the desktop on Windows. Coordinates are global
//! physical pixels (Hex is per-monitor DPI aware), top-left origin at the
//! primary display; a display's `scale` is its DPI scale. Window ids are
//! agent-desktop's `w-<hwnd>`.
//!
//! The screenshot is the display's pixels through GDI, so windows that
//! asked to be left out of captures (`WDA_EXCLUDEFROMCAPTURE`, as Hex's HUD
//! and overlays do) are never in it; what shows of Hex's other windows and
//! of blocked apps' windows is blacked out, and so are password fields.
//! There is nothing to grant: any app may read the screen and UI
//! Automation.

mod uia;

use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use scrawl_capture::win;
use scrawl_capture::windows::RawWindow;

use super::displays::{Display, Frame};
use super::mask::{self, Layer};
use super::{LONG_SIDE, Permissions, Screenshot, encode_png, fit, resize};
use crate::assistant::{Context, TaskWindow};

/// How long reading the selection may take.
const SELECTION_BUDGET: Duration = Duration::from_millis(250);
/// How long finding password fields may take, for all windows together.
const SECURE_BUDGET: Duration = Duration::from_millis(250);

pub fn screenshot() -> Result<Screenshot> {
    let displays = super::displays();
    let display = displays
        .iter()
        .find(|display| display.has_focus)
        .or_else(|| displays.first())
        .ok_or_else(|| eyre!("No display to take a screenshot of"))?;
    let (x, y, w, h) = display.frame;
    let frame = [x, y, w, h];
    let windows = scrawl_capture::windows::raw_windows();
    let shown: Vec<&RawWindow> = windows
        .iter()
        .filter(|window| window.alpha > 0.0 && overlaps(window.bounds, frame))
        .collect();
    let blocked = super::privacy::blocked_pids_now();
    let own = std::process::id() as i32;
    let layers: Vec<Layer> = shown
        .iter()
        .map(|window| Layer {
            rect: window.bounds,
            hidden: blocked.contains(&window.pid)
                || (window.pid == own && !excluded_from_capture(window.id)),
            opaque: window.layer == 0,
        })
        .collect();
    let mut covered = mask::hidden_parts(&layers);
    let app_windows: Vec<isize> = shown
        .iter()
        .filter(|window| window.layer == 0 && window.pid != own && !blocked.contains(&window.pid))
        .map(|window| window.id as isize)
        .collect();
    covered.extend(uia::password_fields(app_windows, SECURE_BUDGET));
    let (mut rgba, width, height) = capture(frame)?;
    super::privacy::black_out(&mut rgba, (width, height), frame, &covered);
    let (fitted_width, fitted_height) = fit(width, height, LONG_SIDE);
    let rgba = resize(&rgba, (width, height), (fitted_width, fitted_height));
    Ok(Screenshot {
        png: encode_png(fitted_width, fitted_height, &rgba)?,
        width: fitted_width,
        height: fitted_height,
    })
}

fn overlaps(bounds: [f64; 4], frame: [f64; 4]) -> bool {
    bounds[0] < frame[0] + frame[2]
        && bounds[0] + bounds[2] > frame[0]
        && bounds[1] < frame[1] + frame[3]
        && bounds[1] + bounds[3] > frame[1]
}

/// Whether Hex's window `id` asked to be left out of captures, so the
/// screenshot never had it.
fn excluded_from_capture(id: u32) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    let mut affinity = 0_u32;
    // SAFETY: Reads one of this process's own window's affinity into a
    // local.
    let read = unsafe { GetWindowDisplayAffinity(id as usize as *mut _, &mut affinity) };
    read != 0 && affinity == WDA_EXCLUDEFROMCAPTURE
}

/// The pixels of `frame` (global pixels), as RGBA, and their size.
fn capture(frame: [f64; 4]) -> Result<(Vec<u8>, u32, u32)> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    let [x, y, w, h] = frame.map(|value| value.round() as i32);
    if w <= 0 || h <= 0 {
        return Err(eyre!("The display has no size"));
    }
    let (width, height) = (w as u32, h as u32);
    let mut bgra = vec![0_u8; width as usize * height as usize * 4];
    // SAFETY: Each GDI object made here is released below; the bitmap is
    // deselected before GetDIBits reads it into `bgra`, which holds a
    // top-down 32-bit image of `width` × `height`.
    let lines = unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err(eyre!("Couldn't take a screenshot: no screen to read"));
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(memory, bitmap);
        let copied = BitBlt(memory, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        SelectObject(memory, previous);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let lines = if copied != 0 {
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
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
        lines
    };
    if lines != h {
        return Err(eyre!("Couldn't take a screenshot of the display"));
    }
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok((bgra, width, height))
}

pub fn window_frame(window: &TaskWindow) -> Option<Frame> {
    super::frame_among(&scrawl_capture::windows::raw_windows(), window)
}

/// The app in front (the last one before Hex, while Hex's own window is),
/// its window's title, the window, and its selected text.
pub fn quick_context() -> Context {
    let front = scrawl_capture::capture::frontmost_app_info();
    let windows = scrawl_capture::windows::raw_windows();
    let foreground = win::foreground_window();
    let focused = front.as_ref().and_then(|(pid, ..)| {
        foreground
            .as_ref()
            .filter(|window| window.pid as i32 == *pid)
            .map(|window| window.hwnd as u32)
    });
    let window = front
        .as_ref()
        .and_then(|(pid, name, _)| super::front_window_among(&windows, *pid, name, focused));
    let window_title = window
        .as_ref()
        .and_then(|window| window.window_id.as_deref())
        .and_then(super::window_number)
        .and_then(|number| windows.iter().find(|raw| raw.id == number))
        .map(|raw| raw.title.clone())
        .filter(|title| !title.is_empty());
    let selected_text = front
        .is_some()
        .then(|| uia::selected_text(SELECTION_BUDGET))
        .flatten()
        .map(super::clip_selection);
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

/// Nothing to grant on Windows.
pub fn permissions() -> Permissions {
    Permissions {
        accessibility: true,
        screen_recording: true,
    }
}

/// The displays, primary first, named by their place ("Display 1"), in
/// global pixels; their ids are those places.
pub fn read_displays() -> Vec<Display> {
    win::monitors()
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| {
            let [x, y, w, h] = monitor.rect;
            let [vx, vy, vw, vh] = monitor.work;
            Display {
                id: (index + 1).to_string(),
                name: format!("Display {}", index + 1),
                frame: (x, y, w, h),
                visible_frame: (vx, vy, vw, vh),
                scale: monitor.scale,
                is_primary: monitor.primary,
                has_focus: false,
            }
        })
        .collect()
}

/// The window in front's frame, and the pointer.
pub fn focus_hints() -> (Option<Frame>, Option<(f64, f64)>) {
    let window = win::foreground_window()
        .filter(|window| window.on_screen())
        .map(|window| {
            let [x, y, w, h] = window.rect;
            (x, y, w, h)
        });
    (window, win::cursor_position())
}

/// The processes with windows, and their executables' names
/// (`1password.exe`), for the blocked-apps check.
pub fn window_owners() -> Vec<(i32, String)> {
    scrawl_capture::windows::raw_windows()
        .into_iter()
        .map(|window| (window.pid, window.owner))
        .collect()
}

pub fn executable_name(pid: i32) -> Option<String> {
    win::app_identity(u32::try_from(pid).ok()?).map(|(_, id)| id)
}
