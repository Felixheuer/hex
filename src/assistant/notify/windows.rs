//! Toasts on Windows, under Hex's AppUserModelID (`anomaly.hex`), which Hex
//! registers for the current user (`HKCU\Software\Classes\AppUserModelId`)
//! with its name and icon: an unpackaged app needs no shortcut for that.
//! Clicking a toast while Hex runs opens Chats on its conversation
//! ([`super::set_on_click`]); Hex keeps the last toasts it showed, since a
//! toast whose object is gone tells nobody it was clicked.

use std::collections::VecDeque;
use std::sync::Mutex;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::{HSTRING, IInspectable};

use super::Note;
use super::payload::{APP_ID, toast_xml};

/// How many shown toasts are kept for their clicks.
const KEPT: usize = 20;

static SHOWN: Mutex<VecDeque<ToastNotification>> = Mutex::new(VecDeque::new());

/// Shows `note` on a thread of its own; returns at once.
pub fn show(note: &Note) {
    let note = note.clone();
    let spawned = std::thread::Builder::new()
        .name("hex-notification".into())
        .spawn(move || {
            use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
            // SAFETY: This thread's own apartment, before any WinRT call.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(error) = post(&note) {
                tracing::warn!(%error, "couldn't show a notification");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't show a notification");
    }
}

fn post(note: &Note) -> windows::core::Result<()> {
    register();
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(note)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    let conversation = note.conversation.clone();
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, _| {
            super::clicked(&conversation);
            Ok(())
        },
    ))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
    let mut shown = SHOWN.lock().unwrap_or_else(|error| error.into_inner());
    shown.push_back(toast);
    while shown.len() > KEPT {
        shown.pop_front();
    }
    Ok(())
}

/// Registers Hex's AppUserModelID with its name and icon, once a run.
fn register() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        if let Err(error) = write_registration() {
            tracing::warn!(%error, "couldn't register Hex for notifications");
        }
    });
}

fn write_registration() -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let icon = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("hex")
        .join("notification-icon.png");
    if let Some(folder) = icon.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(
        &icon,
        include_bytes!("../../../resources/windows/icon-128.png"),
    )?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))?;
    key.set_value("DisplayName", &"Hex")?;
    key.set_value("IconUri", &icon.to_string_lossy().into_owned())?;
    Ok(())
}
