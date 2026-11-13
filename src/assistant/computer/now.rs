//! The desktop right now, for every task's prompt: which apps run (the
//! ones in the Dock), which of them is in front or hidden, and each one's
//! windows with agent-desktop's ids, on this screen or not (on another
//! desktop, minimized). Read in Agent-Desk's own process from the window server
//! and the workspace, in milliseconds, so a turn never starts from what an
//! older turn or another task said was open.
//!
//! macOS reads NSWorkspace's running apps and CoreGraphics' whole window
//! list (`kCGWindowListOptionAll`, which includes windows on other Spaces
//! and minimized ones); elsewhere nothing is read yet and the prompt simply
//! has no such section.

use std::time::{Duration, Instant};

/// Apps the prompt names, at most; the rest are counted.
const MAX_APPS: usize = 30;
/// Windows per app the prompt names, at most.
const MAX_WINDOWS: usize = 6;
/// A window title's length in the prompt, at most (characters).
const MAX_TITLE: usize = 60;
/// The section's size, at most (characters, about 900 tokens).
const MAX_CHARS: usize = 3600;

/// One window of a running app.
#[derive(Clone, Debug, PartialEq)]
pub struct AppWindow {
    /// CoreGraphics' window number: agent-desktop's `w-<number>`.
    pub number: u32,
    pub title: String,
    /// On the screen now; else on another desktop (Space) or minimized.
    pub on_screen: bool,
}

/// One running app with a Dock icon.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub name: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    /// The app in front of the person.
    pub front: bool,
    /// Hidden (⌘H): its windows are on no screen until it's shown.
    pub hidden: bool,
    /// Front to back as the window server lists them.
    pub windows: Vec<AppWindow>,
}

/// What [`read`] found, and how long it took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesktopNow {
    pub apps: Vec<RunningApp>,
    pub took: Duration,
}

/// One entry of the window server's list, as [`read`] uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct RawWindow {
    pub number: u32,
    pub pid: i32,
    pub owner: String,
    pub title: String,
    pub layer: i64,
    pub alpha: f64,
    /// x, y, width, height in points.
    pub bounds: [f64; 4],
    pub on_screen: bool,
}

/// Whether `window` is one a person would call a window of its app: a
/// normal window of a real size; off the screen only with a title (apps
/// keep untitled helper windows ordered out).
pub fn real_window(window: &RawWindow) -> bool {
    let [_, _, width, height] = window.bounds;
    window.layer == 0
        && width >= 60.0
        && height >= 40.0
        && if window.on_screen {
            window.alpha > 0.0
        } else {
            !window.title.trim().is_empty()
        }
}

/// The desktop now: the running apps (not Agent-Desk itself) with their windows.
/// A few milliseconds; never blocks on an app.
pub fn read() -> DesktopNow {
    let started = Instant::now();
    let mut apps = running_apps();
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    apps.retain(|app| app.pid != own);
    let windows = all_windows();
    for app in &mut apps {
        app.windows = windows
            .iter()
            .filter(|window| window.pid == app.pid && real_window(window))
            .map(|window| AppWindow {
                number: window.number,
                title: window.title.clone(),
                on_screen: window.on_screen,
            })
            .collect();
    }
    DesktopNow {
        apps,
        took: started.elapsed(),
    }
}

/// [`read`] as the prompt's section ([`describe`]); `None` in tests (they
/// never read the person's desktop) and where nothing can be read.
pub fn for_prompt(blocked: &[String]) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let now = read();
    tracing::debug!(
        apps = now.apps.len(),
        took_ms = now.took.as_secs_f64() * 1000.0,
        "read the desktop for a prompt"
    );
    describe(&now, blocked)
}

/// The prompt's section about the desktop now: the app in front, then every
/// running app with its windows (ids, titles, whether on this screen), so
/// the agent knows what runs without looking and acts on a window by id at
/// once. Titles of `blocked` apps are left out. `None` when nothing was
/// read.
pub fn describe(now: &DesktopNow, blocked: &[String]) -> Option<String> {
    if now.apps.is_empty() {
        return None;
    }
    let is_blocked = |app: &RunningApp| {
        blocked.iter().any(|entry| {
            let entry = entry.trim().to_lowercase();
            !entry.is_empty()
                && (app.name.to_lowercase() == entry
                    || app
                        .bundle_id
                        .as_deref()
                        .is_some_and(|id| id.to_lowercase() == entry))
        })
    };
    let mut apps: Vec<&RunningApp> = now.apps.iter().collect();
    // In front first, then apps with a window on this screen, then the
    // rest, each by name.
    apps.sort_by_key(|app| {
        (
            !app.front,
            !app.windows.iter().any(|window| window.on_screen),
            app.name.to_lowercase(),
        )
    });
    let mut text = String::from(
        "The desktop right now (Agent-Desk read it as this message was sent; it is current: trust it over anything said earlier in this conversation, other tasks' results, or your memory about which apps and windows are open; window ids work with window_id and desktop_claim_window):\n",
    );
    if let Some(front) = apps.iter().find(|app| app.front) {
        let window = front.windows.iter().find(|window| window.on_screen);
        match window {
            Some(window) if !is_blocked(front) => text.push_str(&format!(
                "In front: {}, window {}\n",
                front.name,
                describe_window(window)
            )),
            _ => text.push_str(&format!("In front: {}\n", front.name)),
        }
    }
    text.push_str("Running apps:\n");
    let mut left_out = Vec::new();
    for (index, app) in apps.iter().enumerate() {
        let line = app_line(app, is_blocked(app));
        if index >= MAX_APPS || text.chars().count() + line.chars().count() + 80 > MAX_CHARS {
            left_out.push(app.name.as_str());
            continue;
        }
        text.push_str(&line);
        text.push('\n');
    }
    if !left_out.is_empty() {
        text.push_str(&format!(
            "- also running: {}\n",
            crate::assistant::store::shorten(&left_out.join(", "), 400)
        ));
    }
    text.push_str("No other app is running.");
    Some(text)
}

fn app_line(app: &RunningApp, blocked: bool) -> String {
    let mut flags = Vec::new();
    if app.front {
        flags.push("in front");
    }
    if app.hidden {
        flags.push("hidden");
    }
    if blocked {
        flags.push("blocked: Agent-Desk won't read or operate it");
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" ({})", flags.join(", "))
    };
    if blocked {
        return format!("- {}{flags}", app.name);
    }
    if app.windows.is_empty() {
        return format!(
            "- {}{flags}: no open window (hex_open_app opens one; it never starts a second instance)",
            app.name
        );
    }
    let mut windows: Vec<String> = app
        .windows
        .iter()
        .take(MAX_WINDOWS)
        .map(describe_window)
        .collect();
    if app.windows.len() > MAX_WINDOWS {
        windows.push(format!("{} more", app.windows.len() - MAX_WINDOWS));
    }
    format!("- {}{flags}: {}", app.name, windows.join("; "))
}

fn describe_window(window: &AppWindow) -> String {
    let title = window.title.trim();
    let mut text = format!("w-{}", window.number);
    if !title.is_empty() {
        text.push_str(&format!(
            " “{}”",
            crate::assistant::store::shorten(title, MAX_TITLE)
        ));
    }
    if !window.on_screen {
        text.push_str(" (not on this screen: another desktop or minimized)");
    }
    text
}

/// Every window the window server has, on the screen or not, front to
/// back for those on it.
pub fn all_windows() -> Vec<RawWindow> {
    #[cfg(target_os = "macos")]
    {
        macos::all_windows()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// The running apps with a Dock icon, without their windows.
fn running_apps() -> Vec<RunningApp> {
    #[cfg(target_os = "macos")]
    {
        macos::running_apps()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString};

    use super::{RawWindow, RunningApp};

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const std::ffi::c_void;
    }

    /// `kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements`.
    const ALL_WINDOWS: u32 = 1 << 4;

    pub fn running_apps() -> Vec<RunningApp> {
        autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            let front = workspace
                .frontmostApplication()
                .map(|app| app.processIdentifier());
            let running = workspace.runningApplications();
            running
                .iter()
                .filter(|app| {
                    app.activationPolicy() == NSApplicationActivationPolicy::Regular
                        && !app.isTerminated()
                })
                .map(|app| {
                    let pid = app.processIdentifier();
                    RunningApp {
                        name: app
                            .localizedName()
                            .map(|name| name.to_string())
                            .unwrap_or_default(),
                        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                        pid,
                        front: Some(pid) == front,
                        hidden: app.isHidden(),
                        windows: Vec::new(),
                    }
                })
                .filter(|app| !app.name.is_empty())
                .collect()
        })
    }

    pub fn all_windows() -> Vec<RawWindow> {
        autoreleasepool(|_| {
            // SAFETY: returns a +1 CFArray of CFDictionaries (toll-free
            // bridged to NSArray of NSDictionary) or null.
            let raw = unsafe { CGWindowListCopyWindowInfo(ALL_WINDOWS, 0) };
            if raw.is_null() {
                return Vec::new();
            }
            // SAFETY: a +1 CFArray is an NSArray we now own.
            let Some(list): Option<Retained<NSArray<NSDictionary<NSString, NSObject>>>> =
                (unsafe { Retained::from_raw(raw.cast_mut().cast()) })
            else {
                return Vec::new();
            };
            let key = |name: &str| NSString::from_str(name);
            let (number, pid, owner, title, layer, alpha, bounds, on_screen) = (
                key("kCGWindowNumber"),
                key("kCGWindowOwnerPID"),
                key("kCGWindowOwnerName"),
                key("kCGWindowName"),
                key("kCGWindowLayer"),
                key("kCGWindowAlpha"),
                key("kCGWindowBounds"),
                key("kCGWindowIsOnscreen"),
            );
            let (x, y, width, height) = (key("X"), key("Y"), key("Width"), key("Height"));
            list.iter()
                .filter_map(|window| {
                    let number_of = |key: &NSString| -> Option<Retained<NSNumber>> {
                        window.objectForKey(key)?.downcast::<NSNumber>().ok()
                    };
                    let text_of = |key: &NSString| -> String {
                        window
                            .objectForKey(key)
                            .and_then(|value| value.downcast::<NSString>().ok())
                            .map(|text| text.to_string())
                            .unwrap_or_default()
                    };
                    let frame = window
                        .objectForKey(&bounds)
                        .and_then(|value| value.downcast::<NSDictionary>().ok());
                    let side = |key: &NSString| -> f64 {
                        frame
                            .as_ref()
                            .and_then(|frame| {
                                let key: &AnyObject = key;
                                frame.objectForKey(key)
                            })
                            .and_then(|value| value.downcast::<NSNumber>().ok())
                            .map_or(0.0, |number| number.doubleValue())
                    };
                    Some(RawWindow {
                        number: number_of(&number)?.unsignedIntValue(),
                        pid: number_of(&pid)?.intValue(),
                        owner: text_of(&owner),
                        title: text_of(&title),
                        layer: number_of(&layer).map_or(0, |layer| layer.longLongValue()),
                        alpha: number_of(&alpha).map_or(1.0, |alpha| alpha.doubleValue()),
                        bounds: [side(&x), side(&y), side(&width), side(&height)],
                        on_screen: number_of(&on_screen).is_some_and(|on| on.boolValue()),
                    })
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(number: u32, title: &str, on_screen: bool) -> AppWindow {
        AppWindow {
            number,
            title: title.into(),
            on_screen,
        }
    }

    fn app(name: &str, front: bool, windows: Vec<AppWindow>) -> RunningApp {
        RunningApp {
            name: name.into(),
            bundle_id: Some(format!("com.example.{}", name.to_lowercase())),
            pid: 1,
            front,
            hidden: false,
            windows,
        }
    }

    #[test]
    fn the_desktop_now_names_the_front_every_app_and_where_its_windows_are() {
        let mut spotify = app("Spotify", false, vec![window(77, "Spotify", false)]);
        spotify.hidden = true;
        let now = DesktopNow {
            apps: vec![
                app("WhatsApp", false, Vec::new()),
                app(
                    "Mail",
                    true,
                    vec![window(31, "Inbox", true), window(40, "Re: Offer", false)],
                ),
                spotify,
                app("1Password", false, vec![window(9, "Vault: Private", true)]),
                app("Ghostty", false, vec![window(12, "~/dev", true)]),
            ],
            took: Duration::from_millis(3),
        };
        let text = describe(&now, &["1Password".into()]).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("The desktop right now"), "{text}");
        assert!(lines[0].contains("trust it over anything said earlier"));
        assert_eq!(lines[1], "In front: Mail, window w-31 “Inbox”");
        assert_eq!(lines[2], "Running apps:");
        assert_eq!(
            lines[3],
            "- Mail (in front): w-31 “Inbox”; w-40 “Re: Offer” (not on this screen: another desktop or minimized)"
        );
        // A blocked app's titles stay out.
        assert_eq!(
            lines[4],
            "- 1Password (blocked: Agent-Desk won't read or operate it)"
        );
        assert_eq!(lines[5], "- Ghostty: w-12 “~/dev”");
        assert_eq!(
            lines[6],
            "- Spotify (hidden): w-77 “Spotify” (not on this screen: another desktop or minimized)"
        );
        // Running without a window: closed, not "open on another desktop".
        assert!(lines[7].starts_with("- WhatsApp: no open window"));
        assert_eq!(lines[8], "No other app is running.");
        assert_eq!(describe(&DesktopNow::default(), &[]), None);
    }

    #[test]
    fn the_desktop_now_stays_small_however_much_runs() {
        let apps = (0..80)
            .map(|n| {
                app(
                    &format!("App {n}"),
                    n == 0,
                    (0..20)
                        .map(|w| window(w, &"a long window title ".repeat(10), true))
                        .collect(),
                )
            })
            .collect();
        let text = describe(
            &DesktopNow {
                apps,
                took: Duration::ZERO,
            },
            &[],
        )
        .unwrap();
        assert!(text.chars().count() <= MAX_CHARS + 500, "{}", text.len());
        assert!(text.contains("14 more"), "windows past six are counted");
        assert!(text.contains("- also running: "));
        assert!(text.ends_with("No other app is running."));
    }

    #[test]
    fn only_real_windows_count() {
        let raw = |layer, size: f64, title: &str, on_screen| RawWindow {
            number: 1,
            pid: 1,
            owner: "Mail".into(),
            title: title.into(),
            layer,
            alpha: 1.0,
            bounds: [0.0, 0.0, size, size],
            on_screen,
        };
        assert!(real_window(&raw(0, 500.0, "Inbox", true)));
        assert!(real_window(&raw(0, 500.0, "", true)));
        assert!(real_window(&raw(0, 500.0, "Inbox", false)));
        // Untitled and off the screen: an app's helper window.
        assert!(!real_window(&raw(0, 500.0, "", false)));
        // The menu bar, a status item, a sliver.
        assert!(!real_window(&raw(25, 500.0, "Menubar", true)));
        assert!(!real_window(&raw(0, 10.0, "Inbox", true)));
    }

    /// Live, read-only: this Mac's apps and windows, read in milliseconds.
    /// Skipped where nothing runs with a window (a headless runner).
    #[cfg(target_os = "macos")]
    #[test]
    fn this_macs_desktop_is_read_in_milliseconds() {
        let _ = read();
        let now = read();
        let text = describe(&now, &[]);
        eprintln!(
            "{} apps, {} windows in {:?}; {} characters",
            now.apps.len(),
            now.apps.iter().map(|app| app.windows.len()).sum::<usize>(),
            now.took,
            text.as_deref().map_or(0, str::len)
        );
        if now.apps.is_empty() {
            return;
        }
        assert!(now.took < Duration::from_millis(100), "{:?}", now.took);
    }
}
