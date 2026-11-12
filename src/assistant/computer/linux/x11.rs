//! The X11 desktop as Hex reads it: the windows the window manager stacks
//! (with their frames), the displays RandR reports, and the root window's
//! pixels. Coordinates are the root window's pixels, top-left origin.

use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use scrawl_capture::windows::RawWindow;

use crate::assistant::computer::displays::Display;

struct Desktop {
    connection: RustConnection,
    root: Window,
}

fn connect() -> Option<Desktop> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    Some(Desktop { connection, root })
}

impl Desktop {
    fn atom(&self, name: &[u8]) -> Option<u32> {
        Some(
            self.connection
                .intern_atom(false, name)
                .ok()?
                .reply()
                .ok()?
                .atom,
        )
    }

    fn cardinals(&self, window: Window, property: u32, kind: u32, length: u32) -> Vec<u32> {
        self.connection
            .get_property(false, window, property, kind, 0, length)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(Iterator::collect))
            .unwrap_or_default()
    }

    fn text(&self, window: Window, property: u32, kind: u32) -> Option<String> {
        let reply = self
            .connection
            .get_property(false, window, property, kind, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(String::from_utf8_lossy(&reply.value).into_owned()).filter(|text| !text.is_empty())
    }
}

/// The managed windows, front to back, as [`RawWindow`]s: the X11 window
/// id, its process, executable name, title, and frame (with the window
/// manager's decorations). Minimized windows have no alpha.
pub fn windows() -> Vec<RawWindow> {
    let Some(desktop) = connect() else {
        return Vec::new();
    };
    let atom = |name: &[u8]| desktop.atom(name).unwrap_or_default();
    let (stacking, pid, name, utf8, extents, state, hidden) = (
        atom(b"_NET_CLIENT_LIST_STACKING"),
        atom(b"_NET_WM_PID"),
        atom(b"_NET_WM_NAME"),
        atom(b"UTF8_STRING"),
        atom(b"_NET_FRAME_EXTENTS"),
        atom(b"_NET_WM_STATE"),
        atom(b"_NET_WM_STATE_HIDDEN"),
    );
    let mut clients = desktop.cardinals(desktop.root, stacking, AtomEnum::WINDOW.into(), 4096);
    // Bottom to top as the window manager stacks them.
    clients.reverse();
    clients
        .into_iter()
        .filter_map(|window| {
            let geometry = desktop.connection.get_geometry(window).ok()?.reply().ok()?;
            let origin = desktop
                .connection
                .translate_coordinates(window, desktop.root, 0, 0)
                .ok()?
                .reply()
                .ok()?;
            let frame = desktop.cardinals(window, extents, AtomEnum::CARDINAL.into(), 4);
            let [left, right, top, bottom] = match frame.as_slice() {
                [l, r, t, b] => [*l, *r, *t, *b].map(f64::from),
                _ => [0.0; 4],
            };
            let process = desktop
                .cardinals(window, pid, AtomEnum::CARDINAL.into(), 1)
                .first()
                .copied();
            let minimized = desktop
                .cardinals(window, state, AtomEnum::ATOM.into(), 64)
                .contains(&hidden);
            let title = desktop
                .text(window, name, utf8)
                .or_else(|| desktop.text(window, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
                .unwrap_or_default();
            Some(RawWindow {
                id: window,
                pid: process.and_then(|pid| i32::try_from(pid).ok()).unwrap_or(0),
                owner: process.and_then(super::executable_of).unwrap_or_default(),
                title,
                bounds: [
                    f64::from(origin.dst_x) - left,
                    f64::from(origin.dst_y) - top,
                    f64::from(geometry.width) + left + right,
                    f64::from(geometry.height) + top + bottom,
                ],
                layer: 0,
                alpha: if minimized { 0.0 } else { 1.0 },
            })
        })
        .collect()
}

/// The window the window manager says is active.
pub fn active_window() -> Option<u32> {
    let desktop = connect()?;
    let active = desktop.atom(b"_NET_ACTIVE_WINDOW")?;
    desktop
        .cardinals(desktop.root, active, AtomEnum::WINDOW.into(), 1)
        .first()
        .copied()
        .filter(|window| *window != 0)
}

/// The pointer, in root pixels.
pub fn pointer() -> Option<(f64, f64)> {
    let desktop = connect()?;
    let reply = desktop
        .connection
        .query_pointer(desktop.root)
        .ok()?
        .reply()
        .ok()?;
    Some((f64::from(reply.root_x), f64::from(reply.root_y)))
}

/// The monitors RandR reports, primary first, their visible frames less
/// the panels (`_NET_WORKAREA`).
pub fn displays() -> Vec<Display> {
    let Some(desktop) = connect() else {
        return Vec::new();
    };
    let Some(monitors) = desktop
        .connection
        .randr_get_monitors(desktop.root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return Vec::new();
    };
    let workarea = desktop
        .atom(b"_NET_WORKAREA")
        .map(|atom| desktop.cardinals(desktop.root, atom, AtomEnum::CARDINAL.into(), 4))
        .filter(|area| area.len() == 4)
        .map(|area| [area[0], area[1], area[2], area[3]].map(f64::from));
    let mut displays: Vec<Display> = monitors
        .monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            let frame = [
                f64::from(monitor.x),
                f64::from(monitor.y),
                f64::from(monitor.width),
                f64::from(monitor.height),
            ];
            let visible = workarea
                .and_then(|area| intersection(frame, area))
                .unwrap_or(frame);
            let name = desktop
                .connection
                .get_atom_name(monitor.name)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("Display {}", index + 1));
            Display {
                id: (index + 1).to_string(),
                name,
                frame: (frame[0], frame[1], frame[2], frame[3]),
                visible_frame: (visible[0], visible[1], visible[2], visible[3]),
                scale: 1.0,
                is_primary: monitor.primary,
                has_focus: false,
            }
        })
        .collect();
    displays.sort_by_key(|display| !display.is_primary);
    displays
}

fn intersection(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let left = a[0].max(b[0]);
    let top = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}

/// The root window's pixels in `frame`, as RGBA, and their size. `None`
/// where the server keeps no picture of the screen in the root (a
/// compositing window manager that draws elsewhere answers all black).
pub fn capture(frame: [f64; 4]) -> Option<(Vec<u8>, u32, u32)> {
    let desktop = connect()?;
    let [x, y, w, h] = frame.map(|value| value.round());
    let (width, height) = (u16::try_from(w as i64).ok()?, u16::try_from(h as i64).ok()?);
    let image = desktop
        .connection
        .get_image(
            ImageFormat::Z_PIXMAP,
            desktop.root,
            x as i16,
            y as i16,
            width,
            height,
            !0,
        )
        .ok()?
        .reply()
        .ok()?;
    let pixels = usize::from(width) * usize::from(height);
    if image.data.len() < pixels * 4 {
        return None;
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    let mut lit = false;
    for pixel in image.data.as_chunks::<4>().0.iter().take(pixels) {
        // BGRX on the little-endian servers Hex runs on.
        lit |= pixel[0] | pixel[1] | pixel[2] != 0;
        rgba.extend([pixel[2], pixel[1], pixel[0], 255]);
    }
    lit.then_some((rgba, u32::from(width), u32::from(height)))
}
