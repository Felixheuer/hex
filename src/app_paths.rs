use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use color_eyre::eyre::{Result, eyre};
use tracing_subscriber::fmt::writer::MakeWriterExt;

pub mod legacy;

/// Whether this is a local build (`AGENT_DESK_LOCAL_BUILD=1`): made from a
/// checkout for its owner's own machine, so it never checks for or offers
/// updates.
pub fn is_local_build() -> bool {
    option_env!("AGENT_DESK_LOCAL_BUILD") == Some("1")
}

/// Which macOS app this build is. Each keeps its name, bundle id, URL
/// scheme, data, settings, and user config apart, so they install side by
/// side and never touch each other's permissions:
/// - **Live**: `Agent-Desk.app` (`fheuer.agentdesk`), the published Pilot
///   release that updates itself;
/// - **Dev**: `Agent-Desk Dev.app` (`fheuer.agentdesk.dev`,
///   `AGENT_DESK_DEV_BUILD=1`), the one development build
///   `scripts/install-fork.sh` installs, never updating;
/// - **Test**: `Agent-Desk Test.app` (`fheuer.agentdesk.test`,
///   `AGENT_DESK_TEST_BUILD=1`), the ad-hoc signed test app.
///
/// Each was called Hex before ([`legacy`]); its old folders are taken over
/// once ([`support_dir`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Variant {
    Live,
    Dev,
    Test,
}

// The name, URL scheme, and rival are for the macOS app's Live and Dev
// copies; the other systems install one variant.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl Variant {
    /// The variant `AGENT_DESK_TEST_BUILD` / `AGENT_DESK_DEV_BUILD` (as
    /// compiled in) name.
    pub const fn from_env(test_build: Option<&str>, dev_build: Option<&str>) -> Self {
        if matches!(test_build, Some(value) if value.len() == 1 && value.as_bytes()[0] == b'1') {
            Self::Test
        } else if matches!(dev_build, Some(value) if value.len() == 1 && value.as_bytes()[0] == b'1')
        {
            Self::Dev
        } else {
            Self::Live
        }
    }

    /// The app's name, as menus and Finder show it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Live => "Agent-Desk",
            Self::Dev => "Agent-Desk Dev",
            Self::Test => "Agent-Desk Test",
        }
    }

    pub const fn bundle_id(self) -> &'static str {
        match self {
            Self::Live => "fheuer.agentdesk",
            Self::Dev => "fheuer.agentdesk.dev",
            Self::Test => "fheuer.agentdesk.test",
        }
    }

    /// Its folder under Application Support and `~/.config`, which is also
    /// its URL scheme (`agent-desk-dev://…`).
    pub const fn folder(self) -> &'static str {
        match self {
            Self::Live => "agent-desk",
            Self::Dev => "agent-desk-dev",
            Self::Test => "agent-desk-test",
        }
    }

    pub const fn scheme(self) -> &'static str {
        self.folder()
    }

    /// The names this variant had as Hex, for the one-time migrations.
    pub const fn legacy(self) -> legacy::Identity {
        legacy::identity(self)
    }

    /// Where its finished meetings go under Documents.
    pub const fn meetings_folder(self) -> &'static str {
        match self {
            Self::Live => "Agent-Desk Meetings",
            Self::Dev => "Agent-Desk Dev Meetings",
            Self::Test => "Agent-Desk Test Meetings",
        }
    }

    /// The other app that listens to the same shortcuts and microphone:
    /// Live and Dev take turns (the one starting quits the other); Test
    /// runs beside either.
    pub const fn rival(self) -> Option<Self> {
        match self {
            Self::Live => Some(Self::Dev),
            Self::Dev => Some(Self::Live),
            Self::Test => None,
        }
    }
}

/// This build's variant; always Live off macOS.
pub fn variant() -> Variant {
    if cfg!(target_os = "macos") {
        Variant::from_env(
            option_env!("AGENT_DESK_TEST_BUILD"),
            option_env!("AGENT_DESK_DEV_BUILD"),
        )
    } else {
        Variant::Live
    }
}

/// Whether this is the macOS `Agent-Desk Dev.app` build
/// (`AGENT_DESK_DEV_BUILD=1`).
pub fn is_macos_dev_build() -> bool {
    variant() == Variant::Dev
}

/// The name menus show, so a Dev or Test build running beside Agent-Desk
/// reads as itself.
#[cfg(target_os = "macos")]
pub fn app_name() -> &'static str {
    variant().name()
}

/// The data folder: `~/Library/Application Support/agent-desk` (and
/// `agent-desk-dev`, …), `~/.local/share/agent-desk`, `%APPDATA%\agent-desk`.
/// An older Hex's folder (`…/hex`) is taken over once ([`take_over`]).
pub fn support_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGENT_DESK_APPLICATION_SUPPORT_DIR") {
        return Ok(path.into());
    }
    static SUPPORT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(path) = SUPPORT.get() {
        return Ok(path.clone());
    }
    let data =
        dirs::data_dir().ok_or_else(|| eyre!("application data directory is unavailable"))?;
    let path = take_over(
        &data.join(variant().legacy().folder),
        &data.join(variant().folder()),
        MOVES_LEGACY_FOLDERS,
    );
    Ok(SUPPORT.get_or_init(|| path).clone())
}

/// Whether an older Hex's folders are renamed to Agent-Desk's. On macOS the
/// data folder holds only data. On Linux and Windows a direct install lives
/// inside it (`versions/`, `current`, the systemd unit, the PATH entry, the
/// Start menu all point into it), so there the old folder is used where it is.
const MOVES_LEGACY_FOLDERS: bool = cfg!(target_os = "macos");

/// One-time migration, remove after 2026-12: the folder to use for
/// `current`, given what an older Hex left at `legacy`. Nothing is ever
/// merged or deleted:
/// - only `legacy` exists: with `moves`, it is renamed to `current` (one
///   `rename`, so it is all or nothing); without `moves`, while an older Hex
///   still holds it, or when the rename fails, `legacy` stays in use;
/// - `current` exists (or neither does): `current`, and a `legacy` beside it
///   is left alone.
///
/// Whichever folder an older Hex's data ends up in, its `settings.json` is
/// rewritten to today's key names once ([`rewrite_renamed_settings`]).
pub(crate) fn take_over(legacy: &Path, current: &Path, moves: bool) -> PathBuf {
    if current.exists() || !legacy.exists() {
        return current.to_path_buf();
    }
    let folder = if !moves || legacy_in_use(legacy) {
        legacy.to_path_buf()
    } else {
        let moved = move_once(legacy, current);
        if moved == current {
            tracing::info!(from = %legacy.display(), to = %current.display(), "moved the Hex folder");
        }
        moved
    };
    if let Err(error) = rewrite_renamed_settings(&folder.join("settings.json")) {
        tracing::warn!(%error, "could not rename the old settings keys");
    }
    folder
}

/// One-time migration, remove after 2026-12: settings renamed while the app
/// was Hex, as (section, old key, new key). Their values move to the new key
/// in `settings.json`; nothing reads the old names any more.
const RENAMED_SETTINGS: &[(&str, &str, &str)] = &[
    ("assistant", "background_front_window", "full_control"),
    ("assistant", "agent_space", "park_windows"),
    ("assistant", "physical_input", "background_physical_input"),
    ("meetings", "never_offer", "providers"),
];

/// One-time migration, remove after 2026-12: moves every old key in
/// [`RENAMED_SETTINGS`] to its new name (a new key already there wins), and
/// writes the file back atomically only when something changed. A missing
/// or unreadable file is left as it is.
pub(crate) fn rewrite_renamed_settings(path: &Path) -> std::io::Result<bool> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(false);
    };
    let Ok(mut settings) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(false);
    };
    let mut changed = false;
    for (section, old, new) in RENAMED_SETTINGS {
        let Some(section) = settings
            .get_mut(*section)
            .and_then(serde_json::Value::as_object_mut)
        else {
            continue;
        };
        if let Some(value) = section.remove(*old) {
            section.entry(*new).or_insert(value);
            changed = true;
        }
    }
    if changed {
        let partial = path.with_extension("json.renaming");
        fs::write(&partial, serde_json::to_vec_pretty(&settings)?)?;
        fs::rename(&partial, path)?;
    }
    Ok(changed)
}

/// Whether a running older Hex holds one of its runtime locks in `folder`
/// (`desktop::instance`): its files move only once it quits.
fn legacy_in_use(folder: &Path) -> bool {
    use fs2::FileExt;
    ["listener.lock", "local-api.lock", "service.lock"]
        .iter()
        .filter_map(|name| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(folder.join(name))
                .ok()
        })
        .any(|file| match file.try_lock_exclusive() {
            Ok(()) => {
                let _ = FileExt::unlock(&file);
                false
            }
            Err(_) => true,
        })
}

pub fn logs_dir() -> Result<PathBuf> {
    static LOGS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    machine_local(&LOGS, "logs")
}

/// Where the speech models live (`transcription::models::models_dir`).
pub fn models_dir() -> Result<PathBuf> {
    static MODELS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    machine_local(&MODELS, "models")
}

/// `name` in the support folder, except on Windows, where logs and models
/// belong to this computer: `%LOCALAPPDATA%\hex`, not the Roaming
/// `%APPDATA%\hex`, which managed networks copy to a server at every sign-in.
/// An older Hex's Roaming copy moves over once. While it can't (a running Hex
/// holds its files), the Roaming copy keeps being used, so nothing downloads
/// again.
fn machine_local(cell: &std::sync::OnceLock<PathBuf>, name: &str) -> Result<PathBuf> {
    if let Some(path) = cell.get() {
        return Ok(path.clone());
    }
    let roaming = support_dir()?.join(name);
    let path = match local_support_dir() {
        Some(local) => move_once(&roaming, &local.join(name)),
        None => roaming,
    };
    Ok(cell.get_or_init(|| path).clone())
}

/// `%LOCALAPPDATA%\agent-desk` on Windows (an older Hex's
/// `%LOCALAPPDATA%\hex`, where it exists, stays in use), unless a data folder
/// is chosen for this run (`AGENT_DESK_APPLICATION_SUPPORT_DIR`); elsewhere
/// none.
fn local_support_dir() -> Option<PathBuf> {
    if !cfg!(windows) || std::env::var_os("AGENT_DESK_APPLICATION_SUPPORT_DIR").is_some() {
        return None;
    }
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(|local| {
            let local = PathBuf::from(local);
            take_over(
                &local.join(variant().legacy().folder),
                &local.join(variant().folder()),
                MOVES_LEGACY_FOLDERS,
            )
        })
}

/// `local`, after moving `roaming` there if only `roaming` exists. A move that
/// fails leaves `roaming` in place and in use.
fn move_once(roaming: &Path, local: &Path) -> PathBuf {
    if local.exists() || !roaming.exists() {
        return local.to_path_buf();
    }
    let moved = local
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::rename(roaming, local));
    match moved {
        Ok(()) => local.to_path_buf(),
        Err(_) => roaming.to_path_buf(),
    }
}

/// Creates `directory` readable only by this user, and tightens it if an
/// older Hex left it open. Tightening is best-effort: a folder Hex can't
/// chmod (another owner, a mount that refuses it) is logged, never fatal.
/// Windows folders already inherit the per-user `%APPDATA%` permissions.
pub fn create_private_dir(directory: &Path) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    tighten(directory, 0o700);
    Ok(())
}

/// Opens `path` to append, readable only by this user, tightening a file an
/// older Hex created open (best-effort, as `create_private_dir`).
pub fn open_private_append(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    tighten(path, 0o600);
    Ok(file)
}

#[cfg(unix)]
fn tighten(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
        tracing::warn!(path = %path.display(), %error, "could not make this private");
    }
}

/// Creates the logs directory, appends process diagnostics to `process.log`
/// alongside stderr, installs the global tracing subscriber, and routes Ctrl-C
/// to `shutdown`. Returns the logs directory.
pub fn init_process_logging(shutdown: &'static AtomicBool) -> Result<PathBuf> {
    let log_dir = logs_dir()?;
    create_private_dir(&log_dir)?;
    let process_log = open_private_append(&log_dir.join("process.log"))?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("agent_desk=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr.and(Mutex::new(process_log)))
        .init();
    log_panics();
    ctrlc::set_handler(|| shutdown.store(true, Ordering::Relaxed))?;
    Ok(log_dir)
}

/// Records every panic in `process.log` before the installed report runs.
/// Desktop apps have no console, so the default stderr report is otherwise
/// lost and a crashed worker thread leaves no trace.
fn log_panics() {
    let report = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            thread = std::thread::current().name().unwrap_or("unnamed"),
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "panic: {info}"
        );
        report(info);
    }));
}

pub fn opencode_workspace() -> Result<PathBuf> {
    Ok(support_dir()?.join("opencode"))
}

pub fn local_api_discovery_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("local-api.json"))
}

pub fn personal_commands_status_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("personal-commands.json"))
}

/// The config hash the person allowed to use `fetch` and run programs. It
/// lives here, not in the config folder, so a config cannot allow itself.
pub fn transform_trust_file() -> Result<PathBuf> {
    Ok(support_dir()?.join("transform-trust.json"))
}

pub fn personal_commands_workspace() -> Result<PathBuf> {
    user_config_dir()
}

/// The per-user config folder holding `hex.config.ts` on every OS:
/// `~/.config/hex`, or `~/.config/hex-dev` / `hex-test` for `Hex Dev.app` /
/// `Hex Test.app`.
fn user_config_dir() -> Result<PathBuf> {
    static CONFIG: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(path) = CONFIG.get() {
        return Ok(path.clone());
    }
    let home = home_dir().ok_or_else(|| eyre!("home directory is unavailable"))?;
    // The config folder holds only the person's files, so an older Hex's
    // `~/.config/hex` moves on every system.
    let path = take_over(
        &home.join(".config").join(variant().legacy().folder),
        &user_config_dir_in(&home, variant()),
        true,
    );
    Ok(CONFIG.get_or_init(|| path).clone())
}

/// The home folder. On Windows `USERPROFILE` names it, as `HOME` does
/// elsewhere, so a copy of Hex launched with another profile folder (an
/// isolated benchmark) reads that folder's config; `dirs::home_dir` asks the
/// shell for the signed-in user's own folder instead. Every path Hex derives
/// from the user's folders goes through these, as Annotate's engine does
/// (`scrawl_router::user_dirs`).
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|profile| !profile.is_empty()) {
        return Some(profile.into());
    }
    dirs::home_dir()
}

/// The roaming data folder: `APPDATA` on Windows, else `dirs::data_dir`.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(roaming) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        return Some(roaming.into());
    }
    dirs::data_dir()
}

/// The Desktop folder (Screenshots saves there by default):
/// `dirs::desktop_dir`, else `Desktop` in the home folder.
pub fn desktop_dir() -> PathBuf {
    dirs::desktop_dir()
        .or_else(|| home_dir().map(|home| home.join("Desktop")))
        .unwrap_or_else(|| PathBuf::from("Desktop"))
}

/// The Documents folder: `dirs::document_dir`, or `Documents` in a Windows
/// profile `USERPROFILE` points to that is not the signed-in user's own.
pub fn document_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let (Some(profile), Some(own)) = (home_dir(), dirs::home_dir())
        && !profile
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(own.to_string_lossy().trim_end_matches(['\\', '/']))
    {
        return Some(profile.join("Documents"));
    }
    dirs::document_dir()
}

fn user_config_dir_in(home: &std::path::Path, variant: Variant) -> PathBuf {
    home.join(".config").join(variant.folder())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_roaming_folder_moves_to_the_local_one_once() {
        let root = std::env::temp_dir().join(format!("hex-local-move-{}", std::process::id()));
        let roaming = root.join("Roaming").join("hex").join("models");
        let local = root.join("Local").join("hex").join("models");
        fs::create_dir_all(&roaming).unwrap();
        fs::write(roaming.join("model.gguf"), b"weights").unwrap();

        assert_eq!(move_once(&roaming, &local), local);
        assert_eq!(fs::read(local.join("model.gguf")).unwrap(), b"weights");
        assert!(!roaming.exists());

        // Later runs, and fresh installs, use the local folder as it is.
        assert_eq!(move_once(&roaming, &local), local);
        let fresh = root.join("Fresh").join("models");
        assert_eq!(move_once(&root.join("none"), &fresh), fresh);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_move_that_fails_keeps_the_roaming_folder_in_use() {
        let root = std::env::temp_dir().join(format!("hex-local-stay-{}", std::process::id()));
        let roaming = root.join("Roaming").join("models");
        fs::create_dir_all(&roaming).unwrap();
        // A file where the local parent folder should be makes the move fail.
        fs::write(root.join("Local"), b"").unwrap();

        assert_eq!(
            move_once(&roaming, &root.join("Local").join("models")),
            roaming
        );
        assert!(roaming.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_logs_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("hex-private-dir-{}", std::process::id()));
        let logs = root.join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o755)).unwrap();

        create_private_dir(&logs).unwrap();
        create_private_dir(&root.join("fresh")).unwrap();

        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&logs), 0o700);
        assert_eq!(mode(&root.join("fresh")), 0o700);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_build_has_its_own_config_folder() {
        let home = std::path::Path::new("/Users/alex");
        assert_eq!(
            user_config_dir_in(home, Variant::Live),
            home.join(".config").join("agent-desk")
        );
        assert_eq!(
            user_config_dir_in(home, Variant::Dev),
            home.join(".config").join("agent-desk-dev")
        );
        assert_eq!(
            user_config_dir_in(home, Variant::Test),
            home.join(".config").join("agent-desk-test")
        );
    }

    #[test]
    fn each_variant_has_its_own_name_id_scheme_and_folders() {
        assert_eq!(Variant::from_env(None, None), Variant::Live);
        assert_eq!(Variant::from_env(None, Some("1")), Variant::Dev);
        assert_eq!(Variant::from_env(Some("1"), Some("1")), Variant::Test);
        assert_eq!(Variant::from_env(Some("0"), Some("0")), Variant::Live);
        let all = [Variant::Live, Variant::Dev, Variant::Test];
        let distinct = |f: fn(Variant) -> &'static str| {
            let mut seen: Vec<&str> = all.iter().map(|v| f(*v)).collect();
            seen.sort_unstable();
            seen.dedup();
            seen.len() == all.len()
        };
        assert!(distinct(Variant::name));
        assert!(distinct(Variant::bundle_id));
        assert!(distinct(Variant::scheme));
        assert!(distinct(Variant::meetings_folder));
        assert_eq!(Variant::Live.name(), "Agent-Desk");
        assert_eq!(Variant::Live.bundle_id(), "fheuer.agentdesk");
        assert_eq!(Variant::Dev.name(), "Agent-Desk Dev");
        assert_eq!(Variant::Dev.bundle_id(), "fheuer.agentdesk.dev");
        assert_eq!(Variant::Dev.scheme(), "agent-desk-dev");
        // Live and Dev take turns; Test runs beside either.
        assert_eq!(Variant::Live.rival(), Some(Variant::Dev));
        assert_eq!(Variant::Dev.rival(), Some(Variant::Live));
        assert_eq!(Variant::Test.rival(), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agent-desk-take-over-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_old_hex_folder_moves_to_the_new_name_once() {
        let root = scratch("move");
        let legacy = root.join("hex");
        let current = root.join("agent-desk");
        fs::create_dir_all(legacy.join("logs")).unwrap();
        fs::write(legacy.join("settings.json"), b"{}").unwrap();
        fs::write(legacy.join("logs/process.log"), b"line").unwrap();

        assert_eq!(take_over(&legacy, &current, true), current);
        assert_eq!(fs::read(current.join("settings.json")).unwrap(), b"{}");
        assert_eq!(fs::read(current.join("logs/process.log")).unwrap(), b"line");
        assert!(!legacy.exists());
        // Later launches use the new folder as it is.
        assert_eq!(take_over(&legacy, &current, true), current);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_folders_are_left_alone_when_the_new_one_exists() {
        let root = scratch("both");
        let legacy = root.join("hex");
        let current = root.join("agent-desk");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join("settings.json"), b"old").unwrap();
        fs::write(current.join("settings.json"), b"new").unwrap();

        assert_eq!(take_over(&legacy, &current, true), current);
        assert_eq!(fs::read(legacy.join("settings.json")).unwrap(), b"old");
        assert_eq!(fs::read(current.join("settings.json")).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_moved_settings_take_the_new_key_names_once() {
        let root = scratch("settings");
        let legacy = root.join("hex");
        let current = root.join("agent-desk");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join("settings.json"),
            br#"{"sound_effects":false,
                 "assistant":{"background_front_window":true,"agent_space":false,
                              "physical_input":true,"park_windows":true},
                 "meetings":{"never_offer":[{"provider":"zoom"}]}}"#,
        )
        .unwrap();

        assert_eq!(take_over(&legacy, &current, true), current);
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(current.join("settings.json")).unwrap()).unwrap();
        assert_eq!(saved["sound_effects"], false);
        assert_eq!(saved["assistant"]["full_control"], true);
        assert_eq!(saved["assistant"]["background_physical_input"], true);
        // A new key already there wins over the old one.
        assert_eq!(saved["assistant"]["park_windows"], true);
        assert_eq!(saved["meetings"]["providers"][0]["provider"], "zoom");
        for old in ["background_front_window", "agent_space", "physical_input"] {
            assert!(saved["assistant"].get(old).is_none(), "{old}");
        }
        assert!(saved["meetings"].get("never_offer").is_none());
        // Nothing left to rename: the file is not written again.
        assert!(!rewrite_renamed_settings(&current.join("settings.json")).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_fresh_install_gets_the_new_folder() {
        let root = scratch("fresh");
        let current = root.join("agent-desk");
        assert_eq!(take_over(&root.join("hex"), &current, true), current);
        assert!(!current.exists(), "the caller creates it");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_old_folder_that_cannot_move_stays_in_use() {
        let root = scratch("stay");
        let legacy = root.join("hex");
        fs::create_dir_all(&legacy).unwrap();
        // Linux and Windows installs live inside it: used where it is.
        assert_eq!(take_over(&legacy, &root.join("agent-desk"), false), legacy);
        // A file where the new folder's parent should be makes the rename fail.
        fs::write(root.join("blocked"), b"").unwrap();
        assert_eq!(
            take_over(&legacy, &root.join("blocked").join("agent-desk"), true),
            legacy
        );
        assert!(legacy.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_running_old_hex_keeps_its_folder_until_it_quits() {
        use fs2::FileExt;
        let root = scratch("running");
        let legacy = root.join("hex");
        let current = root.join("agent-desk");
        fs::create_dir_all(&legacy).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(legacy.join("listener.lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        // flock is per open file, so this process's second open sees it held.
        assert_eq!(take_over(&legacy, &current, true), legacy);
        assert!(!current.exists());

        FileExt::unlock(&lock).unwrap();
        drop(lock);
        assert_eq!(take_over(&legacy, &current, true), current);
        assert!(current.join("listener.lock").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
