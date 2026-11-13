//! What an app's bundle says: its URL schemes and documents
//! (`Info.plist`), what its windows are made of, and the link kinds it is
//! the person's default for (LaunchServices).

use super::*;

/// What a bundle's windows are made of.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ui {
    Native,
    Electron,
    Catalyst,
    /// An iPhone or iPad app running on the Mac.
    Mobile,
}

impl Ui {
    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::Native => {
                "native (AppKit): Accessibility reaches its controls, also in parked windows"
            }
            Self::Electron => {
                "Electron (a web page in a desktop frame): Accessibility sees less and acting \
                 in it is slow; prefer its URL schemes, its web version in your browser, or \
                 hex_read_window to read it"
            }
            Self::Catalyst => {
                "Catalyst (an iPad app made for the Mac): Accessibility reaches most controls; \
                 menus and scripting are thin"
            }
            Self::Mobile => {
                "an iPhone or iPad app on the Mac: Accessibility is thin; hex_read_window reads it"
            }
        }
    }
}

/// What an app's bundle declares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bundle {
    pub version: Option<String>,
    pub url_schemes: Vec<String>,
    pub document_types: Vec<String>,
    pub traits: super::super::recovery::Traits,
}

/// [`Bundle`] from an `Info.plist` (as JSON).
pub fn bundle_of(info: &Value) -> Bundle {
    let strings = |value: Option<&Value>| -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let mut url_schemes: Vec<String> = info
        .get("CFBundleURLTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|kind| strings(kind.get("CFBundleURLSchemes")))
        .map(|scheme| scheme.to_lowercase())
        .collect();
    url_schemes.sort();
    url_schemes.dedup();
    let mut document_types: Vec<String> = info
        .get("CFBundleDocumentTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|kind| {
            kind.get("CFBundleTypeName")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| strings(kind.get("LSItemContentTypes")).into_iter().next())
        })
        .collect();
    document_types.dedup();
    document_types.truncate(MAX_DOCUMENT_TYPES);
    let version = ["CFBundleShortVersionString", "CFBundleVersion"]
        .iter()
        .filter_map(|key| info.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    Bundle {
        version: (!version.is_empty()).then_some(version),
        url_schemes,
        document_types,
        traits: super::super::recovery::traits_of(info),
    }
}

/// [`Ui`] from an `Info.plist` and what the bundle holds.
pub fn ui_of(info: &Value, electron_framework: bool, wrapped: bool) -> Ui {
    if wrapped || info.get("LSRequiresIPhoneOS").and_then(Value::as_bool) == Some(true) {
        Ui::Mobile
    } else if electron_framework {
        Ui::Electron
    } else if info.get("UIDeviceFamily").is_some() {
        Ui::Catalyst
    } else {
        Ui::Native
    }
}

/// The URL schemes LaunchServices opens with `bundle_id` by default (from
/// its `LSHandlers`, as JSON): `https` for the person's browser, `mailto`
/// for their mail app, and any other.
pub fn default_schemes(launch_services: &Value, bundle_id: &str) -> Vec<String> {
    let mut schemes: Vec<String> = launch_services
        .get("LSHandlers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|handler| {
            handler
                .get("LSHandlerRoleAll")
                .and_then(Value::as_str)
                .is_some_and(|id| id.eq_ignore_ascii_case(bundle_id))
        })
        .filter_map(|handler| handler.get("LSHandlerURLScheme").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect();
    schemes.sort();
    schemes.dedup();
    schemes
}

/// The person's LaunchServices choices (`LSHandlers`, as JSON): which app
/// opens which kind of link. `None` when never changed from the system's.
pub fn launch_services() -> Option<Value> {
    let path = dirs::home_dir()?
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    plist_json(&path)
}

/// The app which LaunchServices opens `scheme` links with (`https`: the
/// person's browser), from [`launch_services`]; `None` when the person
/// never chose one (the system's default applies).
pub fn handler_for(launch_services: &Value, scheme: &str) -> Option<String> {
    launch_services
        .get("LSHandlers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|handler| {
            handler
                .get("LSHandlerURLScheme")
                .and_then(Value::as_str)
                .is_some_and(|known| known.eq_ignore_ascii_case(scheme))
        })
        .and_then(|handler| handler.get("LSHandlerRoleAll").and_then(Value::as_str))
        .map(str::to_owned)
}

/// The `Info.plist` of the app bundle at `path`, as JSON.
pub fn info_plist(path: &Path) -> Option<Value> {
    plist_json(&path.join("Contents/Info.plist"))
}

pub(super) fn plist_json(path: &Path) -> Option<Value> {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}
