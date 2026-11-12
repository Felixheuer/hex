//! What each agent-desktop tool is (a read anywhere, a read of a window, an
//! action, real input, …) and which windows its arguments point at. Pure:
//! the decisions that need the desktop's state are in `proxy`.

use serde_json::{Map, Value};

/// What a tool does, for the guard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Reads nothing of a window, or what anyone may see (`list-windows`,
    /// `status`, skills, a timed wait): forwarded as is.
    Free,
    /// The clipboard, which is the person's too: Here tasks, or with real
    /// input allowed.
    Clipboard,
    /// Reads one window (snapshot, find, screenshot, get, is, …).
    Observe,
    /// Operates one window without real input.
    Act,
    /// Real pointer input (hover, drag, mouse-*, double-click, …).
    Physical,
    /// Sends keys to an app (`press`).
    Press,
    /// Brings a window in front.
    FocusWindow,
    Launch,
    /// Quits an app with every window it has.
    CloseApp,
    /// Hex's own `desktop_claim_window`.
    Claim,
    /// Hex's own `hex_installed_apps`.
    InstalledApps,
    /// Hex's own `hex_open_app`.
    OpenApp,
    /// Hex's own `hex_displays`.
    Displays,
    /// Never forwarded; the text says why.
    Refused(&'static str),
}

impl Kind {
    /// Whether it changes something on the desktop (and so waits while the
    /// task is paused, and needs the window to be the task's).
    pub fn acts(self) -> bool {
        matches!(
            self,
            Self::Act
                | Self::Physical
                | Self::Press
                | Self::FocusWindow
                | Self::Launch
                | Self::OpenApp
                | Self::CloseApp
        )
    }
}

/// Hex's tool for taking a free window (Background lane).
pub const CLAIM_TOOL: &str = "desktop_claim_window";

/// Hex's list of the apps installed on this Mac.
pub const INSTALLED_APPS_TOOL: &str = "hex_installed_apps";

/// Hex's safer `launch`: opens an app (or a fresh instance of it) and makes
/// its new window the task's (Background lane).
pub const OPEN_APP_TOOL: &str = "hex_open_app";

/// Hex's display layout: which display the person works on, and which one
/// Background tasks' windows go to.
pub const DISPLAYS_TOOL: &str = "hex_displays";

const MANAGED: &str = "Hex manages agent-desktop's sessions, cursors, traces, and permissions; \
this tool isn't available to tasks.";
const HOLD: &str = "Holding keys or buttons down isn't available to tasks; use desktop_press, \
desktop_mouse_click, or desktop_drag.";
const BATCH: &str = "Batches aren't available through Hex: call each tool on its own so Hex can \
check which window it acts in.";
const PERSONS: &str = "Notifications and system surfaces (Dock, menu bar extras, Spotlight) \
belong to the person; tasks may read notifications but not act on them.";

/// Snapshot surfaces that aren't any app's window.
const SYSTEM_SURFACES: &[&str] = &[
    "desktop",
    "taskbar",
    "system-tray",
    "quick-settings",
    "notification-center",
    "dock",
    "spotlight",
    "menu-bar-extras",
    "system-tray-overflow",
    "start-menu",
    "action-center",
];

/// Ref actions without real input.
const REF_ACTIONS: &[&str] = &[
    "click",
    "right_click",
    "type",
    "set_value",
    "clear",
    "focus",
    "select",
    "toggle",
    "check",
    "uncheck",
    "expand",
    "collapse",
    "scroll",
    "scroll_to",
];

/// Window management by `--window-id` or `--app`.
const WINDOW_ACTIONS: &[&str] = &[
    "resize_window",
    "move_window",
    "minimize",
    "maximize",
    "restore",
];

/// Tools that take `window_id`, so the guard can name the window it checked.
pub const TAKES_WINDOW_ID: &[&str] = &[
    "desktop_snapshot",
    "desktop_find",
    "desktop_screenshot",
    "desktop_press",
    "desktop_focus_window",
    "desktop_resize_window",
    "desktop_move_window",
    "desktop_minimize",
    "desktop_maximize",
    "desktop_restore",
];

pub fn kind(tool: &str, args: &Map<String, Value>) -> Kind {
    match tool {
        CLAIM_TOOL => return Kind::Claim,
        INSTALLED_APPS_TOOL => return Kind::InstalledApps,
        OPEN_APP_TOOL => return Kind::OpenApp,
        DISPLAYS_TOOL => return Kind::Displays,
        _ => {}
    }
    let Some(name) = tool.strip_prefix("desktop_") else {
        return Kind::Refused("Hex doesn't know this tool.");
    };
    match name {
        "list_apps" | "list_windows" | "list_displays" | "status" | "version" | "skills_list"
        | "skills_get" | "skills_path" | "list_notifications" => Kind::Free,
        "clipboard_get" | "clipboard_set" | "clipboard_clear" => Kind::Clipboard,
        "wait" => {
            let bound = ["element", "text", "menu", "menu_closed"]
                .iter()
                .any(|key| {
                    args.get(*key)
                        .is_some_and(|value| !value.is_null() && *value != false)
                });
            if bound { Kind::Observe } else { Kind::Free }
        }
        "snapshot" | "find" | "screenshot" | "get" | "is" | "list_surfaces" => Kind::Observe,
        "double_click" | "triple_click" | "hover" | "drag" | "mouse_move" | "mouse_click"
        | "mouse_wheel" => Kind::Physical,
        "press" => Kind::Press,
        "focus_window" => Kind::FocusWindow,
        "launch" => Kind::Launch,
        "close_app" => Kind::CloseApp,
        "key_down" | "key_up" | "mouse_down" | "mouse_up" => Kind::Refused(HOLD),
        "batch" => Kind::Refused(BATCH),
        "open_system_surface"
        | "dismiss_notification"
        | "dismiss_all_notifications"
        | "notification_action" => Kind::Refused(PERSONS),
        name if REF_ACTIONS.contains(&name) || WINDOW_ACTIONS.contains(&name) => Kind::Act,
        name if name.starts_with("session_")
            || name.starts_with("cursor_overlay_")
            || name.starts_with("trace_")
            || name == "permissions" =>
        {
            Kind::Refused(MANAGED)
        }
        _ => Kind::Refused("Hex doesn't know this tool."),
    }
}

/// Where a call points.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// `window_id`.
    Window(String),
    /// `app`, by name.
    App(String),
    /// A ref, by the snapshot it came from.
    Ref(String),
    /// Absolute screen coordinates.
    Point(f64, f64),
    /// Nothing named: agent-desktop picks the window in front.
    Focused,
}

/// The windows `tool` would read or operate, from its arguments; `Err` when
/// they can't be told (a bad ref, a whole-screen capture).
pub fn targets(tool: &str, args: &Map<String, Value>) -> Result<Vec<Target>, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let reference = |key: &str| text(key).map(snapshot_of).transpose();
    let window_or_app = || match (text("window_id"), text("app")) {
        (Some(id), _) => Target::Window(id.to_owned()),
        (None, Some(app)) => Target::App(app.to_owned()),
        (None, None) => Target::Focused,
    };
    let name = tool.strip_prefix("desktop_").unwrap_or(tool);
    let one = |target: Target| Ok(vec![target]);
    match name {
        "snapshot" | "find" => {
            if let Some(surface) = text("surface")
                && SYSTEM_SURFACES.contains(&surface)
            {
                return Err(PERSONS.into());
            }
            match reference("root")? {
                Some(snapshot) => one(Target::Ref(snapshot)),
                None => one(window_or_app()),
            }
        }
        "screenshot" => {
            if args.get("screen").is_some_and(|screen| !screen.is_null()) {
                return Err(
                    "Whole-screen screenshots show the person's windows; screenshot \
                    your own window with window_id."
                        .into(),
                );
            }
            one(window_or_app())
        }
        "press" | "focus_window" | "list_surfaces" | "close_app" => one(window_or_app()),
        name if WINDOW_ACTIONS.contains(&name) => one(window_or_app()),
        "wait" => match reference("element")? {
            Some(snapshot) => one(Target::Ref(snapshot)),
            None => one(match text("app") {
                Some(app) => Target::App(app.to_owned()),
                None => Target::Focused,
            }),
        },
        "hover" => match (reference("ref_id")?, text("xy")) {
            (Some(snapshot), _) => one(Target::Ref(snapshot)),
            (None, Some(xy)) => one(point(xy)?),
            (None, None) => Err("hover needs ref_id or xy.".into()),
        },
        "drag" => {
            let end = |key: &str, xy_key: &str| match (reference(key)?, text(xy_key)) {
                (Some(snapshot), _) => Ok(Target::Ref(snapshot)),
                (None, Some(xy)) => point(xy),
                (None, None) => Err(format!("drag needs {key} or {xy_key}.")),
            };
            Ok(vec![end("from", "from_xy")?, end("to", "to_xy")?])
        }
        "mouse_move" | "mouse_click" => match text("xy") {
            Some(xy) => one(point(xy)?),
            None => Err(format!("{name} needs xy.")),
        },
        "mouse_wheel" => {
            let number = |key: &str| args.get(key).and_then(Value::as_f64);
            match (number("x"), number("y")) {
                (Some(x), Some(y)) => one(Target::Point(x, y)),
                _ => Err("mouse_wheel needs x and y.".into()),
            }
        }
        _ => match reference("ref_id")? {
            Some(snapshot) => one(Target::Ref(snapshot)),
            None => Err(format!(
                "{name} needs a ref_id from a snapshot of your window."
            )),
        },
    }
}

/// The snapshot id in a qualified ref (`@s8f3k2p9:e5` → `s8f3k2p9`).
pub fn snapshot_of(reference: &str) -> Result<String, String> {
    let bare = reference.trim().trim_start_matches('@');
    match bare.split_once(':') {
        Some((snapshot, element)) if !snapshot.is_empty() && element.starts_with('e') => {
            Ok(snapshot.to_owned())
        }
        _ => Err(format!(
            "“{reference}” isn't a qualified ref (@<snapshot_id>:eN); take a snapshot of your \
             window and use its refs."
        )),
    }
}

/// `x,y` (either may be negative, may start with `=`).
fn point(xy: &str) -> Result<Target, String> {
    let xy = xy.trim().trim_start_matches('=');
    let parsed = xy
        .split_once(',')
        .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)));
    match parsed {
        Some((x, y)) => Ok(Target::Point(x, y)),
        None => Err(format!("“{xy}” isn't a point as x,y.")),
    }
}

/// Every snapshot id in `value` (in `snapshot_id` fields and in refs).
pub fn snapshot_ids(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with('@') => {
            if let Ok(snapshot) = snapshot_of(text)
                && !found.contains(&snapshot)
            {
                found.push(snapshot);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| snapshot_ids(item, found)),
        Value::Object(fields) => {
            for (key, field) in fields {
                match (key.as_str(), field) {
                    ("snapshot_id", Value::String(id)) if !found.contains(id) => {
                        found.push(id.clone());
                    }
                    _ => snapshot_ids(field, found),
                }
            }
        }
        _ => {}
    }
}

/// An app name as compared: case, soft hyphens, and spacing don't count
/// ("System\u{ad}einstellungen" is "Systemeinstellungen").
pub fn app_key(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '\u{ad}')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
