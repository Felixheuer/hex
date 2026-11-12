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
            paths.iter().filter_map(|path| bundle_of(path)).collect()
        } else {
            paths
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

/// Every running process's executable.
fn process_paths() -> Vec<PathBuf> {
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
                Some(PathBuf::from(
                    String::from_utf8_lossy(&buffer[..length]).into_owned(),
                ))
            })
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::assistant::computer::process_paths()
    }
}
