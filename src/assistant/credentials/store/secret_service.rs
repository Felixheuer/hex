//! The desktop's keyring through the freedesktop Secret Service on the
//! session bus (GNOME Keyring, KWallet): each key an item in the default
//! collection with the attributes `service` "Hex Assistant" and `account`.
//! Secrets travel in a "plain" session, as libsecret's do, over the
//! person's own session bus. A locked keyring asks the person to unlock it
//! (the keyring's own prompt); without a Secret Service Hex says so and
//! saves nothing.

use std::collections::HashMap;
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};
use futures_util::StreamExt;
use zbus::{
    Connection, MatchRule, MessageStream, ObjectPath, OwnedObjectPath, OwnedVariant, Variant,
};

use super::{SERVICE, refused};
use crate::assistant::backends::os::Os;

const DESTINATION: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";

/// What a call may take; the keyring answers at once.
const CALL_LIMIT: Duration = Duration::from_secs(5);
/// How long the person has to answer an unlock prompt.
const PROMPT_LIMIT: Duration = Duration::from_secs(120);

const NO_KEYRING: &str = "No keyring is running; install and unlock GNOME Keyring or KWallet";

/// A secret as the Secret Service sends it: its session, parameters
/// (none for "plain"), value, and content type.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// Saves `key` (already checked to be plausible) for `account`.
pub fn save(account: &str, key: &str) -> Result<()> {
    block(async {
        let bus = connect().await?;
        let session = open_session(&bus).await?;
        let collection = default_collection(&bus).await?;
        unlock(&bus, vec![collection.clone()]).await?;
        let attributes = attributes(account);
        let label = format!("{SERVICE}: {account}");
        let properties: HashMap<&str, Variant<'_>> = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", Variant::from(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Variant::from(attributes),
            ),
        ]);
        let secret = (
            session.clone(),
            Vec::<u8>::new(),
            key.as_bytes().to_vec(),
            "text/plain",
        );
        let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) = call(
            &bus,
            collection.as_str(),
            COLLECTION_INTERFACE,
            "CreateItem",
            &(properties, secret, true),
        )
        .await
        .map_err(|error| refused(Os::Linux, "save", error))?;
        if !is_none(&prompt) {
            complete(&bus, &prompt).await?;
        }
        Ok(())
    })
}

pub fn load(account: &str) -> Option<String> {
    let found = block(async {
        let bus = connect().await?;
        let session = open_session(&bus).await?;
        let Some(item) = items(&bus, account).await?.into_iter().next() else {
            return Ok(None);
        };
        let reply = bus
            .call_method(
                Some(DESTINATION),
                item.as_str(),
                Some(ITEM_INTERFACE),
                "GetSecret",
                &(session.clone(),),
            )
            .await?;
        // One struct comes back; read it as its four fields.
        let (_, _, value, _): Secret = reply.body().deserialize_ignore_signature()?;
        Ok(String::from_utf8(value).ok())
    });
    found
        .inspect_err(|error| tracing::debug!(%error, "couldn't read a key from the keyring"))
        .ok()
        .flatten()
}

pub fn delete(account: &str) -> Result<()> {
    block(async {
        let bus = connect().await?;
        for item in items(&bus, account).await? {
            let prompt: OwnedObjectPath = call(&bus, item.as_str(), ITEM_INTERFACE, "Delete", &())
                .await
                .map_err(|error| refused(Os::Linux, "delete", error))?;
            if !is_none(&prompt) {
                complete(&bus, &prompt).await?;
            }
        }
        Ok(())
    })
}

/// Runs `work` on a runtime of its own, bounded by the prompt's limit.
fn block<T>(work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| eyre!("{NO_KEYRING} ({error})"))?;
    runtime.block_on(async {
        tokio::time::timeout(PROMPT_LIMIT + CALL_LIMIT, work)
            .await
            .map_err(|_| eyre!("The keyring didn't answer"))?
    })
}

async fn connect() -> Result<Connection> {
    tokio::time::timeout(CALL_LIMIT, Connection::session())
        .await
        .map_err(|_| eyre!(NO_KEYRING))?
        .map_err(|error| eyre!("{NO_KEYRING} ({error})"))
}

/// A "plain" session: the keyring hands secrets over as they are, on the
/// person's own session bus.
async fn open_session(bus: &Connection) -> Result<OwnedObjectPath> {
    let (_, session): (OwnedVariant, OwnedObjectPath) = call(
        bus,
        SERVICE_PATH,
        SERVICE_INTERFACE,
        "OpenSession",
        &("plain", Variant::from("")),
    )
    .await
    .map_err(|error| eyre!("{NO_KEYRING} ({error})"))?;
    Ok(session)
}

/// The default collection (the "login" keyring).
async fn default_collection(bus: &Connection) -> Result<OwnedObjectPath> {
    let collection: OwnedObjectPath = call(
        bus,
        SERVICE_PATH,
        SERVICE_INTERFACE,
        "ReadAlias",
        &("default",),
    )
    .await?;
    if is_none(&collection) {
        return Err(eyre!(
            "The keyring has no default collection; create one in your keyring app"
        ));
    }
    Ok(collection)
}

fn attributes(account: &str) -> HashMap<&'static str, String> {
    HashMap::from([
        ("service", SERVICE.to_owned()),
        ("account", account.to_owned()),
    ])
}

/// `account`'s items, unlocked (asking the person when the keyring is
/// locked).
async fn items(bus: &Connection, account: &str) -> Result<Vec<OwnedObjectPath>> {
    let (mut unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = call(
        bus,
        SERVICE_PATH,
        SERVICE_INTERFACE,
        "SearchItems",
        &(attributes(account),),
    )
    .await
    .map_err(|error| eyre!("{NO_KEYRING} ({error})"))?;
    if !locked.is_empty() {
        unlock(bus, locked.clone()).await?;
        unlocked.extend(locked);
    }
    Ok(unlocked)
}

async fn unlock(bus: &Connection, objects: Vec<OwnedObjectPath>) -> Result<()> {
    let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        call(bus, SERVICE_PATH, SERVICE_INTERFACE, "Unlock", &(objects,)).await?;
    if !is_none(&prompt) {
        complete(bus, &prompt).await?;
    }
    Ok(())
}

/// Shows the keyring's own prompt and waits for the person's answer.
async fn complete(bus: &Connection, prompt: &OwnedObjectPath) -> Result<()> {
    // Subscribed first, so a quick answer isn't missed.
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(PROMPT_INTERFACE)
        .member("Completed")
        .path(prompt.as_str())
        .build()?;
    let mut completed = MessageStream::for_match_rule(rule, bus, None).await?;
    let () = call(bus, prompt.as_str(), PROMPT_INTERFACE, "Prompt", &("",)).await?;
    let answer = tokio::time::timeout(PROMPT_LIMIT, completed.next())
        .await
        .map_err(|_| eyre!("The keyring's prompt wasn't answered"))?
        .ok_or_else(|| eyre!("The keyring went away"))??;
    let (dismissed, _): (bool, OwnedVariant) = answer.body().deserialize()?;
    if dismissed {
        return Err(eyre!("The keyring stayed locked"));
    }
    Ok(())
}

/// The Secret Service's "no object" path, `/`.
fn is_none(path: &OwnedObjectPath) -> bool {
    path.as_str() == "/"
}

async fn call<B, R>(
    bus: &Connection,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> Result<R>
where
    B: serde::Serialize + zbus::Type,
    R: for<'d> serde::Deserialize<'d> + zbus::Type,
{
    let path = ObjectPath::try_from(path)?;
    let reply = tokio::time::timeout(
        CALL_LIMIT,
        bus.call_method(Some(DESTINATION), &path, Some(interface), method, body),
    )
    .await
    .map_err(|_| eyre!("the keyring didn't answer {method}"))??;
    Ok(reply.body().deserialize()?)
}
