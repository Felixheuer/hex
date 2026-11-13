//! What Agent-Desk reads from a Linux desktop for a question: the focused app and
//! window (`linux::context`, X11 or wlroots' foreign toplevels), the
//! selected text (the PRIMARY selection, which every X11 and Wayland app
//! sets as text is selected), the windows and displays (X11), and a
//! screenshot of the display the person works on: X11 grabs the root
//! window itself; Wayland asks `grim` (wlroots compositors) or the desktop
//! portal's Screenshot (GNOME, KDE; the desktop asks once whether Agent-Desk may).
//! Blocked apps' windows are blacked out on X11, where Agent-Desk can see them.

use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window as XWindow};

use super::Screenshot;
use super::displays::{Display, Frame};
use crate::assistant::{Context, TaskWindow};

/// How long the selection's owner may take to hand its text over.
const SELECTION_WAIT: Duration = Duration::from_millis(250);

/// A top-level window on X11.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    pub id: u32,
    pub pid: Option<u32>,
    /// WM_CLASS's class ("firefox", "org.gnome.Nautilus").
    pub class: String,
    pub title: String,
    /// Root-window pixels: `[x, y, width, height]`.
    pub frame: [f64; 4],
}

fn wayland() -> bool {
    crate::linux::session::LinuxSession::detect().is_wayland()
}

/// The windows on X11, front to back (empty on Wayland, which tells an app
/// nothing of other apps' windows).
pub fn windows() -> Vec<Window> {
    if wayland() {
        return Vec::new();
    }
    x11_windows().unwrap_or_default()
}

fn x11_windows() -> Option<Vec<Window>> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|reply| reply.atom)
    };
    let stacking = atom(b"_NET_CLIENT_LIST_STACKING")?;
    let pid_atom = atom(b"_NET_WM_PID")?;
    let name_atom = atom(b"_NET_WM_NAME")?;
    let utf8 = atom(b"UTF8_STRING")?;
    let list = connection
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, 4096)
        .ok()?
        .reply()
        .ok()?;
    let ids: Vec<XWindow> = list.value32()?.collect();
    let mut windows: Vec<Window> = ids
        .into_iter()
        .rev()
        .filter_map(|id| {
            let geometry = connection.get_geometry(id).ok()?.reply().ok()?;
            let origin = connection
                .translate_coordinates(id, root, 0, 0)
                .ok()?
                .reply()
                .ok()?;
            let pid = connection
                .get_property(false, id, pid_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().and_then(|mut values| values.next()));
            let class = connection
                .get_property(false, id, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| {
                    // "instance\0Class\0": the class names the app.
                    let text = String::from_utf8_lossy(&reply.value).into_owned();
                    text.split('\0')
                        .rfind(|part| !part.is_empty())
                        .unwrap_or_default()
                        .to_owned()
                })
                .unwrap_or_default();
            let title = connection
                .get_property(false, id, name_atom, utf8, 0, 1024)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| String::from_utf8(reply.value).ok())
                .unwrap_or_default();
            Some(Window {
                id,
                pid,
                class,
                title,
                frame: [
                    f64::from(origin.dst_x),
                    f64::from(origin.dst_y),
                    f64::from(geometry.width),
                    f64::from(geometry.height),
                ],
            })
        })
        .collect();
    windows.retain(|window| window.frame[2] > 1.0 && window.frame[3] > 1.0);
    Some(windows)
}

/// The active window's id on X11.
fn active_window() -> Option<u32> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let active = connection
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .ok()?
        .reply()
        .ok()?
        .atom;
    connection
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
        .filter(|id| *id != 0)
}

/// The monitors on X11 (RandR), the primary first; empty on Wayland.
pub fn displays() -> Vec<Display> {
    if wayland() {
        return Vec::new();
    }
    let Some(mut displays) = x11_displays() else {
        return Vec::new();
    };
    let focused = active_window().and_then(|id| {
        windows()
            .into_iter()
            .find(|window| window.id == id)
            .map(|window| {
                let [x, y, w, h] = window.frame;
                (x, y, w, h)
            })
    });
    super::displays::mark_focus(&mut displays, focused, pointer());
    displays
}

fn x11_displays() -> Option<Vec<Display>> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let monitors = connection
        .randr_get_monitors(root, true)
        .ok()?
        .reply()
        .ok()?;
    let mut displays: Vec<Display> = monitors
        .monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            let name = connection
                .get_atom_name(monitor.name)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                .unwrap_or_else(|| format!("Display {}", index + 1));
            let frame = (
                f64::from(monitor.x),
                f64::from(monitor.y),
                f64::from(monitor.width),
                f64::from(monitor.height),
            );
            Display {
                id: (index + 1).to_string(),
                name,
                frame,
                visible_frame: frame,
                scale: 1.0,
                is_primary: monitor.primary,
                has_focus: false,
            }
        })
        .collect();
    if !displays.iter().any(|display| display.is_primary)
        && let Some(first) = displays.first_mut()
    {
        first.is_primary = true;
    }
    displays.sort_by_key(|display| !display.is_primary);
    Some(displays)
}

/// The X11 monitors whatever the session (XWayland's on Wayland).
pub fn x11_monitors() -> Vec<Display> {
    x11_displays().unwrap_or_default()
}

/// The pointer on X11, whatever the session.
pub fn x11_pointer() -> Option<(f64, f64)> {
    pointer()
}

/// The active window's frame on X11, whatever the session.
pub fn x11_front_window() -> Option<[f64; 4]> {
    let id = active_window()?;
    x11_windows()?
        .into_iter()
        .find(|window| window.id == id)
        .map(|window| window.frame)
}

/// The pointer on X11.
fn pointer() -> Option<(f64, f64)> {
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let reply = connection.query_pointer(root).ok()?.reply().ok()?;
    Some((f64::from(reply.root_x), f64::from(reply.root_y)))
}

/// Whether `window` belongs to one of `blocked` (names compared without
/// case, by WM_CLASS or the process's name).
fn blocked(window: &Window, blocked: &[String]) -> bool {
    let process = window
        .pid
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|name| name.trim().to_owned())
        .unwrap_or_default();
    let names = [window.class.as_str(), process.as_str()];
    blocked.iter().any(|app| {
        let app = app.trim();
        !app.is_empty()
            && names
                .iter()
                .any(|name| !name.is_empty() && name.eq_ignore_ascii_case(app))
    })
}

/// A screenshot of the display the person works on.
pub fn screenshot() -> Result<Screenshot> {
    let (width, height, rgba) = if wayland() {
        wayland_shot()?
    } else {
        x11_shot()?
    };
    super::screenshot_from(width, height, &rgba)
}

fn x11_shot() -> Result<(u32, u32, Vec<u8>)> {
    use scrawl_capture::linux::x11::{Grabber, Rect};
    let focused = displays().into_iter().find(|display| display.has_focus);
    let area = focused.as_ref().map(|display| {
        let (x, y, width, height) = display.frame;
        Rect {
            x: x as i16,
            y: y as i16,
            width: width as u16,
            height: height as u16,
        }
    });
    let mut grabber =
        Grabber::open(area).map_err(|error| eyre!("Couldn't take a screenshot: {error:#}"))?;
    let rect = grabber.rect;
    let bgrx = grabber
        .grab()
        .map_err(|error| eyre!("Couldn't take a screenshot: {error:#}"))?;
    let mut rgba = Vec::with_capacity(bgrx.len());
    for [b, g, r, _] in bgrx.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[*r, *g, *b, 255]);
    }
    let (width, height) = (u32::from(rect.width), u32::from(rect.height));
    let display = [
        f64::from(rect.x),
        f64::from(rect.y),
        f64::from(rect.width),
        f64::from(rect.height),
    ];
    let blocked_apps = super::privacy::blocked_apps();
    let hidden: Vec<[f64; 4]> = windows()
        .into_iter()
        .filter(|window| blocked(window, &blocked_apps))
        .map(|window| window.frame)
        .collect();
    super::privacy::black_out(&mut rgba, (width, height), display, &hidden);
    Ok((width, height, rgba))
}

fn wayland_shot() -> Result<(u32, u32, Vec<u8>)> {
    if let Some(grim) = crate::assistant::backends::search::find("grim") {
        let output = crate::desktop::background_process::command(grim)
            .args(["-t", "png", "-"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| eyre!("Couldn't start grim: {error}"))?;
        if output.status.success() {
            return decode_png(&output.stdout);
        }
        tracing::debug!("grim took no screenshot; asking the desktop portal");
    }
    portal_shot()
}

/// The desktop portal's Screenshot, without its dialog (`interactive`
/// false): the desktop asks the person once whether Agent-Desk may.
fn portal_shot() -> Result<(u32, u32, Vec<u8>)> {
    use std::collections::HashMap;
    use zbus::Variant;
    let stop = AtomicBool::new(false);
    let mut bus = crate::linux::portal::PortalBus::open("org.freedesktop.portal.Screenshot", &stop)
        .map_err(|error| eyre!("Couldn't reach the desktop portal: {error:#}"))?
        .ok_or_else(|| {
            eyre!("This desktop can't give Agent-Desk a screenshot; install grim or xdg-desktop-portal")
        })?;
    let results = bus.request(
        "Screenshot",
        |token| {
            (
                "",
                HashMap::from([
                    ("handle_token", Variant::from(token.to_owned())),
                    ("interactive", Variant::from(false)),
                ]),
            )
        },
        &stop,
    )?;
    let uri = results
        .get("uri")
        .and_then(|uri| <&str>::try_from(uri).ok())
        .ok_or_else(|| eyre!("The desktop portal gave no screenshot"))?;
    let path = url::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| eyre!("The desktop portal's screenshot isn't a file"))?;
    let bytes = std::fs::read(&path);
    // The portal saves into the Pictures folder; the question's shot is
    // never kept there.
    let _ = std::fs::remove_file(&path);
    decode_png(&bytes?)
}

/// A PNG's pixels as RGBA.
fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| eyre!("Couldn't read the screenshot: {error}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| eyre!("Couldn't read the screenshot: {error}"))?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
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
        png::ColorType::Grayscale => pixels
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::Indexed => return Err(eyre!("Couldn't read the screenshot's colours")),
    };
    Ok((info.width, info.height, rgba))
}

/// The selected text: the PRIMARY selection, within a quarter second.
fn selected_text() -> Option<String> {
    let text = if wayland() {
        let paste = crate::assistant::backends::search::find("wl-paste")?;
        let mut child = crate::desktop::background_process::command(paste)
            .args(["--primary", "--no-newline", "--type", "text"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < SELECTION_WAIT => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let output = child.wait_with_output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())?
    } else {
        let clipboard = x11_clipboard::Clipboard::new().ok()?;
        let atoms = &clipboard.getter.atoms;
        let bytes = clipboard
            .load(
                atoms.primary,
                atoms.utf8_string,
                atoms.property,
                SELECTION_WAIT,
            )
            .ok()?;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let text = text.trim().to_owned();
    (!text.is_empty()).then(|| super::clip_selection(text))
}

/// The focused app, its window, the selected text, and the displays.
pub fn quick_context() -> Context {
    let focused = crate::linux::context::capture(false);
    let displays = super::displays();
    let chosen = super::agent_display_setting();
    let agents = super::agents_display(&displays, chosen.as_deref());
    let window = focused.application.as_ref().map(|app| TaskWindow {
        app: app.clone(),
        window_id: None,
    });
    Context {
        screenshot: None,
        selected_text: selected_text(),
        frontmost_app: focused.application,
        window_title: focused.window_title,
        window,
        display: super::displays::describe(&displays, agents),
    }
}

/// `window`'s frame on X11: its app's frontmost window (by WM_CLASS or
/// process name); `None` on Wayland.
pub fn window_frame(window: &TaskWindow) -> Option<Frame> {
    let app = window.app.trim();
    let found = windows()
        .into_iter()
        .find(|candidate| blocked(candidate, &[app.to_owned()]))?;
    let [x, y, width, height] = found.frame;
    Some((x, y, width, height))
}

/// Every running process's executable (`/proc/*/exe`).
pub fn process_paths() -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .filter_map(|entry| std::fs::read_link(entry.path().join("exe")).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_app_is_known_by_its_class() {
        let window = Window {
            class: "KeePassXC".into(),
            ..Window::default()
        };
        assert!(blocked(&window, &["keepassxc".into()]));
        assert!(!blocked(&window, &["Bitwarden".into(), " ".into()]));
    }

    #[test]
    fn a_png_reads_back_as_rgba() {
        let rgba = [10, 20, 30, 255, 40, 50, 60, 255];
        let png = super::super::encode_png(2, 1, &rgba).expect("png");
        assert_eq!(decode_png(&png).expect("decoded"), (2, 1, rgba.to_vec()));
    }
}
