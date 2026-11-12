//! Notifications on Linux through `org.freedesktop.Notifications` on the
//! session bus (GNOME, KDE, and the notification daemons of other
//! desktops), under Hex's desktop entry. Each has a default action: a click
//! opens Chats on its conversation ([`super::set_on_click`]). One bus
//! connection, on a one-thread runtime of its own, listens for the clicks
//! of the notifications it showed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use zbus::{Connection, MatchRule, MessageStream, Variant, message};

use super::Note;
use super::payload::{DEFAULT_ACTION, freedesktop};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How many shown notifications are remembered for their clicks.
const KEPT: usize = 50;

struct Notifier {
    runtime: tokio::runtime::Runtime,
    connection: Connection,
    /// The conversation of each notification shown, by its id, oldest first.
    shown: Arc<Mutex<Vec<(u32, String)>>>,
    body_markup: bool,
}

/// The notifier, connected on first use; `None` without a session bus or a
/// notification server.
fn notifier() -> Option<&'static Notifier> {
    static NOTIFIER: OnceLock<Option<Notifier>> = OnceLock::new();
    NOTIFIER
        .get_or_init(|| {
            let connected = connect();
            if let Err(error) = &connected {
                tracing::warn!(%error, "notifications aren't available");
            }
            connected.ok()
        })
        .as_ref()
}

fn connect() -> color_eyre::Result<Notifier> {
    use color_eyre::eyre::{WrapErr, eyre};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("hex-notifications")
        .enable_all()
        .build()
        .wrap_err("could not start the notifications runtime")?;
    let shown: Arc<Mutex<Vec<(u32, String)>>> = Arc::default();
    let listening = shown.clone();
    let (connection, body_markup) = runtime.block_on(async move {
        let connection = tokio::time::timeout(CALL_TIMEOUT, Connection::session())
            .await
            .map_err(|_| eyre!("the session bus didn't answer"))??;
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(SERVICE)
            .path(PATH)
            .build()?;
        let mut stream = MessageStream::for_match_rule(rule, &connection, None).await?;
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                signal(&message, &listening);
            }
        });
        let capabilities: Vec<String> = tokio::time::timeout(
            CALL_TIMEOUT,
            connection.call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &()),
        )
        .await
        .map_err(|_| eyre!("the notification server didn't answer"))??
        .body()
        .deserialize()?;
        color_eyre::Result::<_>::Ok((
            connection,
            capabilities
                .iter()
                .any(|capability| capability == "body-markup"),
        ))
    })?;
    Ok(Notifier {
        runtime,
        connection,
        shown,
        body_markup,
    })
}

/// A click (`ActionInvoked` with the default action) opens its chat; a
/// closed notification is forgotten.
fn signal(message: &zbus::Message, shown: &Mutex<Vec<(u32, String)>>) {
    let header = message.header();
    let body = message.body();
    match header.member().map(|member| member.as_str()) {
        Some("ActionInvoked") => {
            let Ok((id, action)) = body.deserialize::<(u32, String)>() else {
                return;
            };
            let conversation = shown
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .find(|(shown, _)| *shown == id)
                .map(|(_, conversation)| conversation.clone());
            if action == DEFAULT_ACTION
                && let Some(conversation) = conversation
            {
                super::clicked(&conversation);
            }
        }
        Some("NotificationClosed") => {
            if let Ok((id, _reason)) = body.deserialize::<(u32, u32)>() {
                shown
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .retain(|(shown, _)| *shown != id);
            }
        }
        _ => {}
    }
}

/// Shows `note`; returns at once.
pub fn show(note: &Note) {
    let note = note.clone();
    // Connecting can take a moment the first time: never on the caller.
    let spawned = std::thread::Builder::new()
        .name("hex-notification".into())
        .spawn(move || {
            let Some(notifier) = notifier() else {
                return;
            };
            let notify = freedesktop(&note, notifier.body_markup);
            let connection = notifier.connection.clone();
            let shown = notifier.shown.clone();
            notifier.runtime.spawn(async move {
                let hints = HashMap::from([("desktop-entry", Variant::from(notify.desktop_entry))]);
                let arguments = (
                    notify.app_name,
                    notify.replaces_id,
                    notify.app_icon,
                    notify.summary.as_str(),
                    notify.body.as_str(),
                    &notify.actions,
                    hints,
                    notify.expire_timeout,
                );
                let reply = tokio::time::timeout(
                    CALL_TIMEOUT,
                    connection.call_method(
                        Some(SERVICE),
                        PATH,
                        Some(SERVICE),
                        "Notify",
                        &arguments,
                    ),
                )
                .await;
                let id = match reply {
                    Ok(Ok(reply)) => reply.body().deserialize::<u32>(),
                    Ok(Err(error)) => Err(error),
                    Err(_) => {
                        tracing::warn!("the notification server didn't answer");
                        return;
                    }
                };
                match id {
                    Ok(id) => {
                        let mut shown = shown.lock().unwrap_or_else(|error| error.into_inner());
                        shown.push((id, note.conversation));
                        let extra = shown.len().saturating_sub(KEPT);
                        shown.drain(..extra);
                    }
                    Err(error) => tracing::warn!(%error, "couldn't show a notification"),
                }
            });
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "couldn't show a notification");
    }
}
