//! What the guard sees of this computer in its own process, in
//! milliseconds rather than through agent-desktop's hundreds: on macOS the
//! windows on screen (CoreGraphics; an agent-desktop window id `w-N` is
//! CoreGraphics window number N), and everywhere the installed apps
//! (`application_catalog`), which of them run (libproc, `/proc`, or the
//! windows' processes), and the displays. On Linux and Windows the windows
//! come from agent-desktop ([`Native`]), whose window ids are its own.

use std::path::PathBuf;

use super::child::{Desktop, Window};
pub use crate::assistant::computer::Display;

/// An installed app.
#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: PathBuf,
}

/// Where the guard looks; a fake in tests.
pub trait Screen: Send {
    /// Every window on screen, front to back, with the window in front
    /// (`focused`) marked.
    fn windows(&mut self) -> Vec<Window>;

    /// The apps installed on this Mac.
    fn installed_apps(&mut self) -> Vec<App> {
        Vec::new()
    }

    /// The bundles (`…/Name.app`) of the apps running now.
    fn running_apps(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// The displays, with the one the person works on marked.
    fn displays(&mut self) -> Vec<Display> {
        Vec::new()
    }

    /// The windows that aren't on the screen now (on another desktop,
    /// minimized, or of a hidden app), as `visible: false`; a window
    /// [`Screen::windows`] lists isn't among them.
    fn off_screen_windows(&mut self) -> Vec<Window> {
        Vec::new()
    }

    /// The processes running the app bundle `bundle` (`…/Name.app`).
    fn app_pids(&mut self, _bundle: &std::path::Path) -> Vec<i64> {
        Vec::new()
    }

    /// App `pid`'s menu items that have a key equivalent: what a shortcut
    /// does in it, for any app (none where they can't be read).
    fn menu_keys(&mut self, _pid: i64) -> Vec<MenuKey> {
        Vec::new()
    }

    /// What Return would act on in app `pid`'s focused window.
    fn return_target(&mut self, _pid: i64) -> ReturnTarget {
        ReturnTarget::default()
    }
}

/// A menu item's shortcut: modifiers (sorted: `cmd`, `ctrl`, `option`,
/// `shift`) and key (a lowercase character, or `delete` for ⌫,
/// `forward_delete`, `return`, `escape`, `tab`, `space`, an arrow).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuKey {
    pub title: String,
    pub modifiers: Vec<&'static str>,
    pub key: String,
    pub enabled: bool,
}

/// What Return would act on: the focused element (whether it takes text,
/// and more than one line of it), the window's default button, and the
/// buttons nearest the element, nearest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReturnTarget {
    pub editable: bool,
    /// `None`: an editable element of unknown kind.
    pub multiline: Option<bool>,
    pub default_button: Option<String>,
    pub near: Vec<String>,
}

impl ReturnTarget {
    /// From an element's role (`AXTextArea`, `textfield`, …).
    pub fn of_role(role: &str) -> Self {
        let (editable, multiline) = input_role(role);
        Self {
            editable,
            multiline,
            ..Self::default()
        }
    }
}

/// Whether a role takes text, and more than one line of it (`None`:
/// unknown), in Accessibility's names or a driver's (`AXTextArea`,
/// `textarea`, `text_field`, …).
pub fn input_role(role: &str) -> (bool, Option<bool>) {
    let flat: String = role
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let flat = flat.strip_prefix("ax").unwrap_or(&flat);
    match flat {
        "textarea" | "textview" | "multilinetextfield" => (true, Some(true)),
        "textfield" | "searchfield" | "combobox" | "securetextfield" | "entry" | "edit" => {
            (true, Some(false))
        }
        _ if flat.contains("text") && (flat.contains("field") || flat.contains("box")) => {
            (true, None)
        }
        _ => (false, None),
    }
}

/// A key equivalent as Accessibility reports it (`AXMenuItemCmdChar`,
/// `…VirtualKey`, `…Glyph`, `…Modifiers`) in [`MenuKey`]'s terms.
pub fn menu_key(
    title: &str,
    cmd_char: Option<&str>,
    virtual_key: Option<i64>,
    glyph: Option<i64>,
    modifiers: i64,
    enabled: bool,
) -> Option<MenuKey> {
    let named = |name: &str| Some(name.to_owned());
    let character = cmd_char.and_then(|text| {
        let mut chars = text.chars();
        let first = chars.next()?;
        chars.next().is_none().then_some(first)
    });
    let key = match character {
        Some(c) if !c.is_control() && c != ' ' => Some(c.to_lowercase().collect()),
        _ => None,
    }
    .or_else(|| match glyph? {
        2 => named("tab"),
        4 | 11 => named("return"),
        9 => named("space"),
        10 => named("forward_delete"),
        23 => named("delete"),
        27 => named("escape"),
        100 => named("left"),
        101 => named("right"),
        104 => named("up"),
        106 => named("down"),
        _ => None,
    })
    .or_else(|| match character? {
        '\r' | '\n' | '\u{3}' => named("return"),
        '\u{8}' | '\u{7f}' => named("delete"),
        '\u{1b}' => named("escape"),
        '\t' => named("tab"),
        ' ' => named("space"),
        _ => None,
    })
    .or_else(|| match virtual_key? {
        36 | 76 => named("return"),
        48 => named("tab"),
        49 => named("space"),
        51 => named("delete"),
        53 => named("escape"),
        117 => named("forward_delete"),
        123 => named("left"),
        124 => named("right"),
        125 => named("down"),
        126 => named("up"),
        _ => None,
    })?;
    let mut held = Vec::new();
    if modifiers & 8 == 0 {
        held.push("cmd");
    }
    if modifiers & 4 != 0 {
        held.push("ctrl");
    }
    if modifiers & 2 != 0 {
        held.push("option");
    }
    if modifiers & 1 != 0 {
        held.push("shift");
    }
    held.sort_unstable();
    Some(MenuKey {
        title: title.trim().to_owned(),
        modifiers: held,
        key,
        enabled,
    })
}

/// This Mac, read in process.
pub struct CoreGraphics;

impl CoreGraphics {
    /// Reads the installed apps and the windows once on a thread of its own,
    /// so the first calls find them read and the frameworks loaded (the
    /// first reads take tens of milliseconds, later ones under one).
    pub fn warm() {
        let _ = std::thread::Builder::new()
            .name("hex-guard-warm".into())
            .spawn(|| {
                let _ = Self.windows();
                // Starts learning the displays' names (system_profiler).
                let _ = Self.displays();
                crate::application_catalog::discover();
            });
    }
}

impl Screen for CoreGraphics {
    fn windows(&mut self) -> Vec<Window> {
        #[cfg(target_os = "macos")]
        {
            let raw = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
            let pid = scrawl_capture::ax::focused_app_pid();
            let number = pid.and_then(crate::accessibility::focused_window_number);
            windows_among(&raw, pid, number)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }

    fn installed_apps(&mut self) -> Vec<App> {
        crate::application_catalog::discover()
            .into_iter()
            .map(|app| App {
                name: app.name,
                bundle_id: app.bundle_id,
                path: app.path,
            })
            .collect()
    }

    fn displays(&mut self) -> Vec<Display> {
        crate::assistant::computer::displays()
    }

    fn running_apps(&mut self) -> Vec<PathBuf> {
        let paths = process_paths();
        if cfg!(target_os = "macos") {
            paths
                .iter()
                .filter_map(|(_, path)| bundle_of(path))
                .collect()
        } else {
            paths.into_iter().map(|(_, path)| path).collect()
        }
    }

    fn off_screen_windows(&mut self) -> Vec<Window> {
        crate::assistant::computer::now::all_windows()
            .into_iter()
            .filter(|window| {
                !window.on_screen && crate::assistant::computer::now::real_window(window)
            })
            .map(|window| {
                let [x, y, width, height] = window.bounds;
                Window {
                    id: format!("w-{}", window.number),
                    app: window.owner,
                    title: window.title,
                    pid: Some(i64::from(window.pid)),
                    focused: false,
                    visible: false,
                    minimized: false,
                    bounds: Some((x, y, width, height)),
                }
            })
            .collect()
    }

    fn app_pids(&mut self, bundle: &std::path::Path) -> Vec<i64> {
        process_paths()
            .into_iter()
            .filter(|(_, path)| bundle_of(path).as_deref() == Some(bundle))
            .filter_map(|(pid, _)| pid)
            .collect()
    }

    fn menu_keys(&mut self, pid: i64) -> Vec<MenuKey> {
        #[cfg(target_os = "macos")]
        {
            let Ok(pid) = i32::try_from(pid) else {
                return Vec::new();
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
            crate::accessibility::keys::menu_keys(pid, deadline)
                .iter()
                .filter_map(|item| {
                    menu_key(
                        &item.title,
                        item.cmd_char.as_deref(),
                        item.virtual_key,
                        item.glyph,
                        item.modifiers,
                        item.enabled,
                    )
                })
                .collect()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = pid;
            Vec::new()
        }
    }

    fn return_target(&mut self, pid: i64) -> ReturnTarget {
        #[cfg(target_os = "macos")]
        {
            let Ok(pid) = i32::try_from(pid) else {
                return ReturnTarget::default();
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
            let focused = crate::accessibility::keys::focused(pid, deadline);
            let role = if focused.subrole == "AXSearchField" {
                &focused.subrole
            } else {
                &focused.role
            };
            ReturnTarget {
                default_button: focused.default_button,
                near: focused.near,
                ..ReturnTarget::of_role(role)
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = pid;
            ReturnTarget::default()
        }
    }
}

/// Linux and Windows: the windows as agent-desktop lists them (its window
/// ids are its own there), everything else read in process as
/// [`CoreGraphics`] reads it.
pub struct Native<D: Desktop>(pub D);

impl<D: Desktop> Screen for Native<D> {
    fn windows(&mut self) -> Vec<Window> {
        listed_windows(&mut self.0)
    }

    fn installed_apps(&mut self) -> Vec<App> {
        CoreGraphics.installed_apps()
    }

    fn running_apps(&mut self) -> Vec<PathBuf> {
        CoreGraphics.running_apps()
    }

    fn displays(&mut self) -> Vec<Display> {
        CoreGraphics.displays()
    }
}

/// agent-desktop's `list-windows`, through its own `agent-desktop mcp`.
pub struct Listed<D: Desktop>(pub D);

impl<D: Desktop> Screen for Listed<D> {
    fn windows(&mut self) -> Vec<Window> {
        listed_windows(&mut self.0)
    }
}

/// agent-desktop's `list-windows`.
fn listed_windows(desktop: &mut impl Desktop) -> Vec<Window> {
    desktop
        .call("desktop_list_windows", &serde_json::Map::new())
        .ok()
        .and_then(|result| super::child::data(&result))
        .map(|data| super::child::windows(&data))
        .unwrap_or_default()
}

/// `raw` (front to back) as agent-desktop would list it: ordinary windows
/// (layer 0, shown, not a sliver) are `visible`. In front is the window
/// the focused app says has focus (`focused_window`), else that app's first
/// ordinary window (without Accessibility to ask, the first of all).
pub fn windows_among(
    raw: &[scrawl_capture::windows::RawWindow],
    focused_pid: Option<i32>,
    focused_window: Option<u32>,
) -> Vec<Window> {
    let ordinary = |window: &scrawl_capture::windows::RawWindow| {
        window.layer == 0 && window.alpha > 0.0 && window.bounds[2] > 1.0 && window.bounds[3] > 1.0
    };
    let front = focused_window
        .filter(|number| raw.iter().any(|window| window.id == *number))
        .or_else(|| {
            raw.iter()
                .filter(|window| ordinary(window))
                .find(|window| focused_pid.is_none_or(|pid| window.pid == pid))
                .map(|window| window.id)
        });
    raw.iter()
        .map(|window| {
            let [x, y, width, height] = window.bounds;
            Window {
                id: format!("w-{}", window.id),
                app: window.owner.clone(),
                title: window.title.clone(),
                pid: Some(i64::from(window.pid)),
                focused: Some(window.id) == front,
                visible: ordinary(window),
                minimized: false,
                bounds: Some((x, y, width, height)),
            }
        })
        .collect()
}

/// The `.app` bundle an executable lives in
/// (`/Applications/Mail.app/Contents/MacOS/Mail` → `/Applications/Mail.app`).
pub fn bundle_of(executable: &std::path::Path) -> Option<PathBuf> {
    let text = executable.to_str()?;
    let end = text.find(".app/Contents/")?;
    Some(PathBuf::from(&text[..end + ".app".len()]))
}

/// Every running process's executable, with its pid where known.
fn process_paths() -> Vec<(Option<i64>, PathBuf)> {
    #[cfg(target_os = "macos")]
    {
        let mut pids = vec![0_i32; 8192];
        let bytes = i32::try_from(pids.len() * std::mem::size_of::<i32>()).unwrap_or(i32::MAX);
        // SAFETY: `pids` has room for `bytes` bytes of pids.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        let count = usize::try_from(count).unwrap_or(0).min(pids.len());
        pids[..count]
            .iter()
            .filter(|pid| **pid > 0)
            .filter_map(|pid| {
                let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
                // SAFETY: `buffer` holds PROC_PIDPATHINFO_MAXSIZE bytes.
                let length = unsafe {
                    libc::proc_pidpath(*pid, buffer.as_mut_ptr().cast(), buffer.len() as u32)
                };
                let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
                Some((
                    Some(i64::from(*pid)),
                    PathBuf::from(String::from_utf8_lossy(&buffer[..length]).into_owned()),
                ))
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::assistant::computer::process_paths()
            .into_iter()
            .map(|path| (None, path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_equivalents_read_as_combos() {
        // ⇧⌘D: the character, with shift.
        let send = menu_key("Send", Some("D"), None, None, 1, true).unwrap();
        assert_eq!(send.modifiers, ["cmd", "shift"]);
        assert_eq!(send.key, "d");
        // ⌘⌫ comes as a glyph, a virtual key, or a control character.
        for (cmd_char, key, glyph) in [
            (None, None, Some(23)),
            (None, Some(51), None),
            (Some("\u{8}"), None, None),
        ] {
            let trash = menu_key("Move to Trash", cmd_char, key, glyph, 0, true).unwrap();
            assert_eq!(trash.modifiers, ["cmd"]);
            assert_eq!(trash.key, "delete");
        }
        // ⌥⌘⎋, and a key without ⌘ (8): plain ⌫.
        let quit = menu_key("Force Quit…", None, None, Some(27), 2, true).unwrap();
        assert_eq!(quit.modifiers, ["cmd", "option"]);
        assert_eq!(quit.key, "escape");
        let plain = menu_key("Delete", None, None, Some(23), 8, false).unwrap();
        assert!(plain.modifiers.is_empty() && !plain.enabled);
        assert_eq!(plain.key, "delete");
        assert_eq!(menu_key("Nothing", None, None, None, 0, true), None);
    }

    #[test]
    fn roles_tell_one_line_from_many() {
        assert_eq!(input_role("AXTextArea"), (true, Some(true)));
        assert_eq!(input_role("textarea"), (true, Some(true)));
        assert_eq!(input_role("AXTextField"), (true, Some(false)));
        assert_eq!(input_role("text_field"), (true, Some(false)));
        assert_eq!(input_role("AXSearchField"), (true, Some(false)));
        assert_eq!(input_role("rich text box"), (true, None));
        assert_eq!(input_role("AXButton"), (false, None));
        assert_eq!(input_role(""), (false, None));
    }
}
