//! Notifications for Background jobs (`assistant.notifications`): a job
//! finished, failed, or asks something, and clicking one opens the
//! Assistant pane on its conversation ([`set_open`]).
//!
//! ```ts
//! macOS    UNUserNotificationCenter (the `assistant_notifications` Swift bridge) inside
//!          the signed Agent-Desk.app; `osascript` outside a bundle (a click opens nothing)
//! Linux    the desktop's notification server (`notify-send --action --wait`)
//! Windows  a toast (Windows.UI.Notifications) under Agent-Desk's own app id
//! ```
//!
//! OWNER: brain.

use std::sync::Mutex;

/// What a click on a notification opens (set by the window's owner).
type Open = Box<dyn Fn(&str) + Send + Sync>;

static OPEN: Mutex<Option<Open>> = Mutex::new(None);

/// Sets what a click on a notification does with its conversation: open
/// Chats on it. Called on a notification's own thread, so post to the UI
/// and return at once.
pub fn set_open(open: impl Fn(&str) + Send + Sync + 'static) {
    *OPEN.lock().unwrap_or_else(|error| error.into_inner()) = Some(Box::new(open));
}

/// A click on the notification for `conversation`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn clicked(conversation: &str) {
    match OPEN
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        Some(open) => open(conversation),
        None => tracing::debug!("a notification was clicked before Chats could open"),
    }
}

/// What a notification says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// The conversation a click opens.
    pub conversation: String,
}

/// The notification for a Background job's event, if it gets one.
pub fn for_event(event: &crate::assistant::Event, title: &str) -> Option<Note> {
    use crate::assistant::Event;
    let (heading, body, conversation) = match event {
        Event::Answer {
            conversation, text, ..
        } => ("Done", text.as_str(), conversation),
        Event::Failed {
            conversation,
            message,
        } => ("Didn't finish", message.as_str(), conversation),
        Event::Question { conversation, text } => {
            ("Needs your answer", text.as_str(), conversation)
        }
        _ => return None,
    };
    Some(Note {
        title: format!("{heading}: {}", crate::assistant::store::shorten(title, 50)),
        body: crate::assistant::store::shorten(body, 200),
        conversation: conversation.clone(),
    })
}

/// Whether a job's event gets a notification: notifications are on, the job
/// runs in the background, it wasn't typed in Chats (whose answer shows right
/// there), and Agent-Desk isn't the app in front (Chats or another Agent-Desk window shows
/// it).
pub fn wanted(enabled: bool, background: bool, typed_in_chats: bool, hex_in_front: bool) -> bool {
    enabled && background && !typed_in_chats && !hex_in_front
}

/// Whether one of Agent-Desk's own windows is in front.
pub fn hex_in_front() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2::rc::autoreleasepool(|_| {
            objc2_app_kit::NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .is_some_and(|app| app.processIdentifier() == std::process::id() as i32)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Shows `note`; returns at once.
pub fn show(note: &Note) {
    #[cfg(target_os = "macos")]
    {
        if bridge::post(note) {
            return;
        }
        let script = format!(
            "display notification {} with title {}",
            applescript(&note.body),
            applescript(&note.title)
        );
        let spawned = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => tracing::warn!(%error, "couldn't show a notification"),
        }
    }
    #[cfg(target_os = "linux")]
    linux::show(note.clone());
    #[cfg(target_os = "windows")]
    windows::show(note.clone());
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let _ = note;
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// How long a shown notification's click is waited for; a server that
    /// keeps it forever (GNOME's list) stops being watched then.
    const WAIT: Duration = Duration::from_secs(30 * 60);

    /// Whether this `notify-send` takes actions (libnotify 0.7.9 and later).
    fn takes_actions(program: &std::path::Path) -> bool {
        static ACTIONS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ACTIONS.get_or_init(|| {
            crate::desktop::background_process::command(program)
                .arg("--help")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains("--action"))
        })
    }

    /// Shows `note` through `notify-send`, and opens its conversation when
    /// it's clicked.
    pub(super) fn show(note: super::Note) {
        let Some(program) = crate::assistant::backends::search::find("notify-send") else {
            tracing::info!("no notify-send, so the job's notification isn't shown");
            return;
        };
        let actions = takes_actions(&program);
        let mut command = crate::desktop::background_process::command(&program);
        command.args(["--app-name=Agent-Desk", "--icon=dialog-information"]);
        if actions {
            command.arg(format!(
                "--action=default={}",
                crate::desktop::i18n::tr("Show in Assistant")
            ));
            command.arg("--wait");
        }
        // "--" keeps a title or body that starts with "-" from being an
        // option.
        command
            .arg("--")
            .arg(&note.title)
            .arg(&note.body)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-notification".into())
            .spawn(move || {
                let mut child = match command.spawn() {
                    Ok(child) => child,
                    Err(error) => {
                        tracing::warn!(%error, "couldn't show a notification");
                        return;
                    }
                };
                let started = Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if started.elapsed() < WAIT => {
                            std::thread::sleep(Duration::from_millis(250));
                        }
                        _ => {
                            let _ = child.kill();
                            let _ = child.wait();
                            return;
                        }
                    }
                }
                let mut said = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut said);
                }
                if actions && said.trim() == "default" {
                    super::clicked(&note.conversation);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "couldn't show a notification");
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::sync::Mutex;

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::{HSTRING, IInspectable};

    /// Agent-Desk's app id for its toasts: registered under
    /// `HKCU\Software\Classes\AppUserModelId`, which names the sender
    /// "Agent-Desk" for an app that isn't packaged.
    const APP_ID: &str = "fheuer.AgentDesk";

    /// The toasts on screen, kept so their click handlers stay alive.
    static SHOWN: Mutex<Vec<ToastNotification>> = Mutex::new(Vec::new());

    pub(super) fn show(note: super::Note) {
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-notification".into())
            .spawn(move || {
                if let Err(error) = toast(&note) {
                    tracing::warn!(%error, "couldn't show a notification");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "couldn't show a notification");
        }
    }

    fn toast(note: &super::Note) -> windows::core::Result<()> {
        register();
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
            escape(&note.title),
            escape(&note.body)
        )))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        let conversation = note.conversation.clone();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                super::clicked(&conversation);
                Ok(())
            },
        ))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?
            .Show(&toast)?;
        let mut shown = SHOWN.lock().unwrap_or_else(|error| error.into_inner());
        // The newest few are all a person can still click.
        if shown.len() >= 16 {
            shown.remove(0);
        }
        shown.push(toast);
        Ok(())
    }

    /// Names Agent-Desk as the toasts' sender, once a run.
    fn register() {
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| {
            let key = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
            let registered = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                .create_subkey(&key)
                .and_then(|(key, _)| {
                    key.set_value("DisplayName", &"Agent-Desk")?;
                    if let Ok(exe) = std::env::current_exe() {
                        key.set_value("IconUri", &exe.to_string_lossy().into_owned())?;
                    }
                    Ok(())
                });
            if let Err(error) = registered {
                tracing::warn!(%error, "couldn't register Agent-Desk's notification sender");
            }
        });
    }

    /// `text` safe inside XML.
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

/// `text` as an AppleScript string literal.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn applescript(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
mod bridge {
    use std::ffi::CString;

    unsafe extern "C" {
        fn hex_assistant_notifications_available() -> bool;
        fn hex_assistant_notify(
            title: *const std::ffi::c_char,
            body: *const std::ffi::c_char,
            conversation: *const std::ffi::c_char,
        ) -> bool;
    }

    /// Posts through UNUserNotificationCenter; false outside a bundle.
    pub(super) fn post(note: &super::Note) -> bool {
        // SAFETY: takes no arguments; reads the main bundle.
        if !unsafe { hex_assistant_notifications_available() } {
            return false;
        }
        let clean = |text: &str| CString::new(text.replace('\0', "")).unwrap_or_default();
        let (title, body, conversation) = (
            clean(&note.title),
            clean(&note.body),
            clean(&note.conversation),
        );
        // SAFETY: three valid NUL-terminated strings that outlive the call;
        // the bridge copies them before returning.
        unsafe { hex_assistant_notify(title.as_ptr(), body.as_ptr(), conversation.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::Event;

    #[test]
    fn a_job_notifies_its_answer_failure_or_question() {
        let note = for_event(
            &Event::Question {
                conversation: "c1".into(),
                text: "Who is your colleague?".into(),
            },
            "Supplier research",
        )
        .unwrap();
        assert_eq!(
            note,
            Note {
                title: "Needs your answer: Supplier research".into(),
                body: "Who is your colleague?".into(),
                conversation: "c1".into(),
            }
        );
        assert!(
            for_event(
                &Event::Thinking {
                    conversation: "c1".into()
                },
                "x"
            )
            .is_none()
        );
        assert_eq!(applescript(r#"say "hi" \ ok"#), r#""say \"hi\" \\ ok""#);
    }

    #[test]
    fn only_background_jobs_away_from_hex_notify() {
        assert!(wanted(true, true, false, false));
        assert!(!wanted(false, true, false, false), "turned off");
        assert!(!wanted(true, false, false, false), "a Here question");
        assert!(!wanted(true, true, true, false), "typed in Chats");
        assert!(!wanted(true, true, false, true), "Agent-Desk is in front");
    }
}
