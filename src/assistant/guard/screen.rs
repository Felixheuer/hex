//! What the guard sees of this computer in its own process, in
//! milliseconds rather than through agent-desktop's hundreds: the windows
//! on screen, the installed apps (`application_catalog`), and which of them
//! run.
//!
//! - macOS: CoreGraphics' windows (an agent-desktop window id `w-N` is
//!   CoreGraphics window number N); running apps by their `.app` bundles
//!   (libproc).
//! - Windows: the top-level windows (`w-<hwnd>`, as agent-desktop names
//!   them); running apps by their executables.
//! - Linux: agent-desktop names windows by their accessible addresses,
//!   which only it makes, so the guard asks its `list-windows` ([`Listed`]);
//!   running apps by their executables' names (`/proc`).
//!
//! An app's "bundle" off macOS is what the application catalog keeps as its
//! path: its executable on Windows, its desktop entry on Linux.

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

    /// The apps installed on this computer.
    fn installed_apps(&mut self) -> Vec<App> {
        Vec::new()
    }

    /// The installed apps running now, by their paths: `…/Name.app` on
    /// macOS, as [`App::path`] names them elsewhere.
    fn running_apps(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// The displays, with the one the person works on marked.
    fn displays(&mut self) -> Vec<Display> {
        Vec::new()
    }
}

/// This computer, read in process.
pub struct Native;

impl Native {
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

impl Screen for Native {
    /// On Linux the windows come from agent-desktop ([`Listed`]); this
    /// lists none there.
    fn windows(&mut self) -> Vec<Window> {
        #[cfg(target_os = "macos")]
        {
            let raw = objc2::rc::autoreleasepool(|_| scrawl_capture::windows::raw_windows());
            let pid = scrawl_capture::ax::focused_app_pid();
            let number = pid.and_then(crate::accessibility::focused_window_number);
            windows_among(&raw, pid, number)
        }
        #[cfg(target_os = "windows")]
        {
            let raw = scrawl_capture::windows::raw_windows();
            let front = scrawl_capture::win::foreground_window();
            windows_among(
                &raw,
                front.as_ref().map(|window| window.pid as i32),
                front.map(|window| window.hwnd as u32),
            )
        }
        #[cfg(target_os = "linux")]
        {
            Vec::new()
        }
    }

    fn installed_apps(&mut self) -> Vec<App> {
        installed_apps()
    }

    fn displays(&mut self) -> Vec<Display> {
        crate::assistant::computer::displays()
    }

    fn running_apps(&mut self) -> Vec<PathBuf> {
        running_apps()
    }
}

fn installed_apps() -> Vec<App> {
    crate::application_catalog::discover()
        .into_iter()
        .map(|app| App {
            name: app.name,
            bundle_id: app.bundle_id,
            path: app.path,
        })
        .collect()
}

fn running_apps() -> Vec<PathBuf> {
    let running = process_paths();
    if cfg!(target_os = "macos") {
        running.iter().filter_map(|path| bundle_of(path)).collect()
    } else {
        running_among(&installed_apps(), &running)
    }
}

/// agent-desktop's `list-windows`, through its own `agent-desktop mcp`; the
/// apps and displays as [`Native`] reads them.
pub struct Listed<D: Desktop>(pub D);

impl<D: Desktop> Screen for Listed<D> {
    fn windows(&mut self) -> Vec<Window> {
        self.0
            .call("desktop_list_windows", &serde_json::Map::new())
            .ok()
            .and_then(|result| super::child::data(&result))
            .map(|data| super::child::windows(&data))
            .unwrap_or_default()
    }

    fn installed_apps(&mut self) -> Vec<App> {
        installed_apps()
    }

    fn displays(&mut self) -> Vec<Display> {
        crate::assistant::computer::displays()
    }

    fn running_apps(&mut self) -> Vec<PathBuf> {
        running_apps()
    }
}

/// `raw` (front to back) as agent-desktop would list it: ordinary windows
/// (layer 0, shown, not a sliver) are `visible`. In front is the window
/// the focused app says has focus (`focused_window`), else that app's first
/// ordinary window (without Accessibility to ask, the first of all).
#[cfg(any(test, not(target_os = "linux")))]
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

/// The installed apps among the `running` executables, by the path the
/// catalog keeps for them: the same executable (Windows paths ignore case),
/// or the executable the app is named after (Linux names an app by its
/// executable, `firefox`; Windows by its stem, `chrome`).
pub fn running_among(apps: &[App], running: &[PathBuf]) -> Vec<PathBuf> {
    let stem = |path: &std::path::Path| {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    let running_stems: std::collections::HashSet<String> =
        running.iter().map(|path| stem(path)).collect();
    apps.iter()
        .filter(|app| {
            running.iter().any(|path| {
                path.to_string_lossy()
                    .eq_ignore_ascii_case(&app.path.to_string_lossy())
            }) || running_stems.contains(&app.name.to_lowercase())
        })
        .map(|app| app.path.clone())
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
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
        let mut pids = vec![0_u32; 8192];
        let mut bytes = 0_u32;
        // SAFETY: `pids` has room for the bytes passed; `bytes` gets how many
        // were written.
        let listed = unsafe {
            EnumProcesses(
                pids.as_mut_ptr(),
                (pids.len() * std::mem::size_of::<u32>()) as u32,
                &mut bytes,
            )
        };
        if listed == 0 {
            return Vec::new();
        }
        pids.truncate(bytes as usize / std::mem::size_of::<u32>());
        pids.into_iter()
            .filter(|pid| *pid != 0)
            .filter_map(scrawl_capture::win::process_path)
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().parse::<u32>().is_ok())
            .filter_map(|entry| std::fs::read_link(entry.path().join("exe")).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, path: &str) -> App {
        App {
            name: name.into(),
            bundle_id: None,
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn running_apps_are_the_catalogs_paths_off_macos() {
        let apps = [
            app(
                "chrome",
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            ),
            app("firefox", "/usr/share/applications/firefox.desktop"),
            app("code", "/usr/share/applications/code.desktop"),
        ];
        let running = [
            PathBuf::from(r"c:\program files\google\chrome\application\chrome.exe"),
            PathBuf::from("/usr/lib/firefox/firefox"),
        ];
        assert_eq!(
            running_among(&apps, &running),
            [apps[0].path.clone(), apps[1].path.clone()]
        );
        assert!(running_among(&apps, &[]).is_empty());
    }
}
