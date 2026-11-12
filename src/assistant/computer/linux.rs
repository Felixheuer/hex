//! Hex's own reads of the desktop on Linux.
//!
//! - X11: the windows the window manager stacks and RandR's displays, in
//!   root pixels; the screenshot is the root window's pixels, with what
//!   shows of Hex's and blocked apps' windows blacked out, and password
//!   fields too where their toolkit tells their place. A compositor that
//!   keeps no picture in the root (all black) falls back to the portal.
//! - Wayland: the outputs' logical layout, and a screenshot through the
//!   Screenshot portal ([`wayland`]); other windows' places aren't known,
//!   so task outlines have no frame there.
//!
//! The selection and password fields come through AT-SPI ([`accessible`]),
//! which is the one thing to turn on (`org.a11y.Status.IsEnabled`, and
//! GNOME's `toolkit-accessibility`); on Wayland the screenshot portal asks
//! once whether Hex may take screenshots.

mod accessible;
mod wayland;
mod x11;

use std::time::Duration;

use color_eyre::{Result, eyre::eyre};

use super::displays::{Display, Frame};
use super::mask::{self, Layer};
use super::{LONG_SIDE, Permission, Permissions, Screenshot, encode_png, fit, resize};
use crate::assistant::{Context, TaskWindow};
use crate::linux::atspi::WindowHint;

/// How long reading the selection may take.
const SELECTION_BUDGET: Duration = Duration::from_millis(250);
/// How long finding password fields may take.
const SECURE_BUDGET: Duration = Duration::from_millis(250);
/// How long naming an agent-desktop window may take.
const WINDOW_BUDGET: Duration = Duration::from_millis(150);

fn wayland() -> bool {
    crate::linux::session::LinuxSession::detect().is_wayland()
}

pub fn screenshot() -> Result<Screenshot> {
    let (rgba, width, height) = if wayland() {
        portal_screenshot()?
    } else {
        x11_screenshot().or_else(|error| {
            tracing::debug!(error = %format!("{error:#}"), "no X11 screenshot; asking the portal");
            portal_screenshot()
        })?
    };
    let (fitted_width, fitted_height) = fit(width, height, LONG_SIDE);
    let rgba = resize(&rgba, (width, height), (fitted_width, fitted_height));
    Ok(Screenshot {
        png: encode_png(fitted_width, fitted_height, &rgba)?,
        width: fitted_width,
        height: fitted_height,
    })
}

/// The display the person works on, from the root window, with what must
/// not show blacked out.
fn x11_screenshot() -> Result<(Vec<u8>, u32, u32)> {
    let displays = super::displays();
    let display = displays
        .iter()
        .find(|display| display.has_focus)
        .or_else(|| displays.first())
        .ok_or_else(|| eyre!("No display to take a screenshot of"))?;
    let (x, y, w, h) = display.frame;
    let frame = [x, y, w, h];
    let windows = x11::windows();
    let blocked = super::privacy::blocked_pids_now();
    let own = std::process::id() as i32;
    let layers: Vec<Layer> = windows
        .iter()
        .filter(|window| window.alpha > 0.0)
        .map(|window| Layer {
            rect: window.bounds,
            hidden: window.pid == own || blocked.contains(&window.pid),
            opaque: true,
        })
        .collect();
    let mut covered = mask::hidden_parts(&layers);
    if let Some(hint) = focused_hint() {
        covered.extend(accessible::password_fields(&hint, SECURE_BUDGET));
    }
    let (mut rgba, width, height) =
        x11::capture(frame).ok_or_else(|| eyre!("The X server keeps no picture of the screen"))?;
    super::privacy::black_out(&mut rgba, (width, height), frame, &covered);
    Ok((rgba, width, height))
}

/// The portal's picture of the desktop, cut to the display the person
/// works on when the layout is known. Not while a blocked app runs: its
/// windows couldn't be left out.
fn portal_screenshot() -> Result<(Vec<u8>, u32, u32)> {
    if blocked_app_running() {
        return Err(eyre!(
            "A blocked app is open, and Wayland can't leave its windows out of a screenshot"
        ));
    }
    let (rgba, width, height) = wayland::screenshot()?;
    let displays = super::displays();
    let crop = displays
        .iter()
        .find(|display| display.has_focus)
        .and_then(|display| super::crop_rect(&displays, display, (width, height)));
    Ok(match crop {
        Some(rect) => (super::crop(&rgba, width, rect), rect[2], rect[3]),
        None => (rgba, width, height),
    })
}

/// Whether a process of a blocked app runs (by its executable's name).
fn blocked_app_running() -> bool {
    let blocked = super::privacy::blocked_apps();
    if blocked.is_empty() {
        return false;
    }
    let owners: Vec<(i32, String)> = processes()
        .into_iter()
        .filter_map(|pid| Some((pid, executable_of(u32::try_from(pid).ok()?)?)))
        .collect();
    !super::privacy::blocked_pids(&owners, |_| None, &blocked).is_empty()
}

fn processes() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

/// A process's executable name, lowercased, as the application catalog
/// names apps (`firefox`).
pub fn executable_of(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|path| Some(path.file_name()?.to_str()?.to_owned()))
        .or_else(|| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|name| name.trim().trim_end_matches(" (deleted)").to_lowercase())
        .filter(|name| !name.is_empty())
}

/// The focused window as a hint for AT-SPI.
fn focused_hint() -> Option<WindowHint> {
    let focused = crate::linux::context::focused()?;
    Some(WindowHint {
        pid: focused.pid,
        title: focused.title,
    })
}

/// Where `window` is, in root pixels (X11 only: Wayland tells no app where
/// other windows are). An agent-desktop id is the window's accessible
/// address; its process and title find the X11 window.
pub fn window_frame(window: &TaskWindow) -> Option<Frame> {
    if wayland() {
        return None;
    }
    let windows = x11::windows();
    let named = window
        .window_id
        .as_deref()
        .and_then(|id| accessible::window_of(id, WINDOW_BUDGET));
    let found = match named {
        Some((pid, title)) => windows.iter().find(|raw| {
            raw.alpha > 0.0
                && pid.is_none_or(|pid| raw.pid == pid as i32)
                && title.as_ref().is_none_or(|title| raw.title == *title)
        }),
        None => windows
            .iter()
            .find(|raw| raw.alpha > 0.0 && raw.owner.eq_ignore_ascii_case(window.app.trim())),
    }?;
    let [x, y, w, h] = found.bounds;
    Some((x, y, w, h))
}

/// The app in front, its window's title, and its selected text. The window
/// itself is named only by its app: agent-desktop's window ids are
/// accessible addresses Hex doesn't make.
pub fn quick_context() -> Context {
    let focused = crate::linux::context::focused();
    let selected_text = focused
        .as_ref()
        .and_then(|focused| {
            accessible::selected_text(
                &WindowHint {
                    pid: focused.pid,
                    title: focused.title.clone(),
                },
                SELECTION_BUDGET,
            )
        })
        .map(super::clip_selection);
    let displays = super::displays();
    let chosen = super::agent_display_setting();
    let agents = super::agents_display(&displays, chosen.as_deref());
    let window = focused
        .as_ref()
        .and_then(|focused| focused.application.clone())
        .map(|app| TaskWindow {
            app,
            window_id: None,
        });
    Context {
        screenshot: None,
        selected_text,
        frontmost_app: focused
            .as_ref()
            .and_then(|focused| focused.application.clone()),
        window_title: focused.and_then(|focused| focused.title),
        window,
        display: super::displays::describe(&displays, agents),
    }
}

/// The accessibility bus, and (on Wayland) the screenshot portal's answer;
/// X11 needs no permission for screenshots.
pub fn permissions() -> Permissions {
    Permissions {
        accessibility: accessible::enabled(Duration::from_secs(1)),
        screen_recording: !wayland() || wayland::allowed(),
    }
}

/// Accessibility: turns the accessibility bus on (and GNOME's
/// `toolkit-accessibility`), at once. Screen Recording on Wayland: asks the
/// portal for a screenshot, so it asks its question now.
pub fn open_settings(permission: Permission) {
    match permission {
        Permission::Accessibility => {
            std::thread::spawn(|| {
                if !accessible::enable(Duration::from_secs(2)) {
                    tracing::warn!("couldn't turn the accessibility bus on");
                }
                if let Err(error) = crate::linux::gsettings::gsettings(&[
                    "set",
                    "org.gnome.desktop.interface",
                    "toolkit-accessibility",
                    "true",
                ]) {
                    tracing::debug!(%error, "no GNOME accessibility setting to turn on");
                }
            });
        }
        Permission::ScreenRecording if wayland() => wayland::ask(),
        Permission::ScreenRecording => {}
    }
}

/// The displays: RandR's on X11, the outputs' logical layout on Wayland.
pub fn read_displays() -> Vec<Display> {
    if wayland() {
        wayland::displays()
    } else {
        x11::displays()
    }
}

/// The focused window's frame and the pointer (X11 only).
pub fn focus_hints() -> (Option<Frame>, Option<(f64, f64)>) {
    if wayland() {
        return (None, None);
    }
    let window = x11::active_window().and_then(|active| {
        x11::windows()
            .into_iter()
            .find(|window| window.id == active)
            .map(|window| {
                let [x, y, w, h] = window.bounds;
                (x, y, w, h)
            })
    });
    (window, x11::pointer())
}

/// The processes with windows (X11), named by their executables.
pub fn window_owners() -> Vec<(i32, String)> {
    if wayland() {
        return Vec::new();
    }
    x11::windows()
        .into_iter()
        .map(|window| (window.pid, window.owner))
        .collect()
}

pub fn executable_name(pid: i32) -> Option<String> {
    executable_of(u32::try_from(pid).ok()?)
}
