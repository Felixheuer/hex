//! What the media tools share: where their results go (the task's own
//! folder, else Agent-Desk's media folder), how a path the agent names is
//! read, programs run with a time limit, and images handed back inline.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::assistant::backends::run;

/// The longest side of an image sent inline: larger ones go as a preview.
const INLINE_EDGE: u32 = 1568;

/// The folder results go to: the task's workspace when it has one, else
/// `media-out` in Agent-Desk's assistant folder.
pub fn folder(task: Option<&str>) -> Result<PathBuf, String> {
    if let Some(folder) = task
        .and_then(|task| {
            crate::assistant::store::Store::open()
                .ok()?
                .load(task)?
                .workspace
        })
        .filter(|folder| folder.is_dir())
    {
        return Ok(folder);
    }
    let root = crate::assistant::store::assistant_dir()
        .map_err(|error| format!("{error:#}"))?
        .join("media-out");
    crate::app_paths::create_private_dir(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

/// `path` with `~/` and `file://` resolved; a relative one is in `base`.
pub fn resolve(path: &str, base: &Path) -> PathBuf {
    let path = path.trim();
    let path = path.strip_prefix("file://").unwrap_or(path);
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = crate::app_paths::home_dir()
    {
        return home.join(rest);
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Whom this process's media tools serve: a job or helper reads pages the
/// person never saw, so its paths are held tighter (`set_job`; the tools
/// server sets it once, per task).
static JOB: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Called once by the tools server with its role.
pub fn set_job(job: bool) {
    let _ = JOB.set(job);
}

pub fn serves_a_job() -> bool {
    // Unset (another caller): held as tight as a job.
    JOB.get().copied().unwrap_or(true)
}

/// Agent-Desk's own folders, where a tool may write and overwrite: the
/// task's folder (`base`), Agent-Desk's folder in Documents (every job and
/// project), and the temp folder.
fn own_folders(base: &Path) -> Vec<PathBuf> {
    let mut folders = vec![base.to_path_buf(), std::env::temp_dir()];
    folders.extend(crate::assistant::agent::workspace::root());
    folders.extend(
        crate::assistant::store::assistant_dir()
            .ok()
            .map(|dir| dir.join("media-out")),
    );
    folders.into_iter().map(|folder| real(&folder)).collect()
}

/// The person's own folders, where a tool may add a new file but never
/// replace one.
fn person_folders() -> Vec<PathBuf> {
    let Some(home) = crate::app_paths::home_dir() else {
        return Vec::new();
    };
    [
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Movies",
        "Music",
    ]
    .iter()
    .map(|name| real(&home.join(name)))
    .collect()
}

/// `path` with its links followed as far as it exists: the nearest
/// existing folder made real, the rest joined on.
fn real(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path.to_path_buf();
    loop {
        if let Ok(found) = at.canonicalize() {
            let mut real = found;
            for part in rest.iter().rev() {
                real.push(part);
            }
            return real;
        }
        match (at.file_name().map(|name| name.to_owned()), at.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                at = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Whether `path` (made real) is where secrets live: a dot-folder or
/// dot-file in the home folder (`.ssh`, `.config`, `.agent-browser`, the
/// agents' tokens) or its `Library` (apps' data, cookies, keychains), but
/// iCloud Drive and cloud storage folders there.
fn private(path: &Path) -> bool {
    crate::app_paths::home_dir().is_some_and(|home| private_in(path, &real(&home)))
}

fn private_in(path: &Path, home: &Path) -> bool {
    let Ok(inside) = path.strip_prefix(home) else {
        return false;
    };
    let parts: Vec<String> = inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.iter().any(|part| part.starts_with('.')) {
        return true;
    }
    parts.first().is_some_and(|first| first == "Library")
        && !matches!(
            parts.get(1).map(String::as_str),
            Some("Mobile Documents" | "CloudStorage")
        )
}

/// Why a tool may not read `path`, if it may not.
pub fn may_read(path: &Path, base: &Path) -> Result<(), String> {
    let real = real(path);
    if own_folders(base).iter().any(|own| real.starts_with(own)) || !private(&real) {
        return Ok(());
    }
    Err(format!(
        "“{}” is private (a hidden folder or Library): Agent-Desk's tools don't read there.",
        path.display()
    ))
}

/// Why a tool may not write `path`, if it may not: anywhere in Agent-Desk's
/// own folders, a new file in the person's own folders (not for a job:
/// it reads pages the person never saw, so it adds there only when the
/// file is new too), never through a link, and nowhere else.
pub fn may_write(path: &Path, base: &Path) -> Result<(), String> {
    let refused = |why: &str| {
        Err(format!(
            "Agent-Desk's tools don't write “{}”: {why}. Save it in the task's folder, or under a new name in Documents, Desktop or Downloads.",
            path.display()
        ))
    };
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return refused("the path leaves its folder");
    }
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return refused("it is a link");
    }
    let real = real(path);
    if own_folders(base).iter().any(|own| real.starts_with(own)) {
        return Ok(());
    }
    if private(&real) {
        return refused("it is private (a hidden folder or Library)");
    }
    if person_folders()
        .iter()
        .any(|folder| real.starts_with(folder))
    {
        return if path.exists() {
            refused("a file of the person's is there already")
        } else {
            Ok(())
        };
    }
    refused("it is outside the task's folder and the person's own folders")
}

/// An existing file the agent named (argument `key`).
pub fn input(arguments: &Value, key: &str, base: &Path, tool: &str) -> Result<PathBuf, String> {
    let named = arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("{tool} needs “{key}”: a file's path"))?;
    let path = resolve(named, base);
    if !path.is_file() {
        return Err(format!("There is no file “{named}”."));
    }
    may_read(&path, base)?;
    Ok(path)
}

/// Where a result goes: argument `key` (absolute, or relative to `base`),
/// else `default` in `base`; its folder is made.
pub fn output(arguments: &Value, key: &str, base: &Path, default: &str) -> Result<PathBuf, String> {
    let path = match arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        Some(named) => resolve(named, base),
        None => base.join(default),
    };
    may_write(&path, base)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Couldn't make {}: {error}", parent.display()))?;
    }
    Ok(path)
}

/// `path`'s extension, lowercase.
pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// `stem` with `extension`, unique in `base` (`name-2.ext`, …).
pub fn fresh_name(base: &Path, stem: &str, extension: &str) -> String {
    let first = format!("{stem}.{extension}");
    if !base.join(&first).exists() {
        return first;
    }
    (2..)
        .map(|n| format!("{stem}-{n}.{extension}"))
        .find(|name| !base.join(name).exists())
        .unwrap_or(first)
}

/// Runs `command` for at most `limit`; its output, or why it failed.
pub fn run(command: Command, limit: Duration, what: &str) -> Result<run::Ran, String> {
    let ran = run::bounded(command, limit).map_err(|error| format!("{what}: {error}"))?;
    if ran.timed_out() {
        return Err(format!("{what} took more than {} s", limit.as_secs()));
    }
    if !ran.success() {
        return Err(format!("{what} failed: {}", ran.last_words()));
    }
    Ok(ran)
}

/// ffmpeg with Agent-Desk's quiet defaults.
pub fn ffmpeg_command(ffmpeg: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    command
}

/// Seconds since `since`, to a tenth.
pub fn took(since: Instant) -> f64 {
    (since.elapsed().as_secs_f64() * 10.0).round() / 10.0
}

/// The MCP image content for the image at `path`, scaled down to
/// [`INLINE_EDGE`] when larger (as JPEG); `None` if it can't be read.
pub fn inline_image(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    let mime = match extension(path).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "",
    };
    let small = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .is_some_and(|(width, height)| width.max(height) <= INLINE_EDGE);
    let (mime, data) = if small && !mime.is_empty() {
        (mime, bytes)
    } else {
        let decoded = image::load_from_memory(&bytes).ok()?;
        let preview = decoded.thumbnail(INLINE_EDGE, INLINE_EDGE).to_rgb8();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode_image(&preview)
            .ok()?;
        ("image/jpeg", jpeg)
    };
    Some(json!({"type": "image", "mimeType": mime,
        "data": base64::engine::general_purpose::STANDARD.encode(data)}))
}

/// `report` as the text content, then up to `max` of `images` inline.
pub fn content(report: &Value, images: &[PathBuf], max: usize) -> Vec<Value> {
    let mut content = vec![json!({"type": "text", "text": report.to_string()})];
    content.extend(
        images
            .iter()
            .take(max)
            .filter_map(|path| inline_image(path)),
    );
    content
}

/// A time like `1:20`, `0:01:20.5`, or `80` in seconds.
pub fn seconds(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().filter(|seconds| *seconds >= 0.0),
        Value::String(text) => {
            let text = text.trim().trim_end_matches('s');
            text.parse::<f64>()
                .ok()
                .or_else(|| super::probe::clock_seconds(text))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_in_the_folder_and_names_stay_fresh() {
        let base = std::env::temp_dir().join(format!("hex-media-out-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(resolve("a/b.png", &base), base.join("a/b.png"));
        assert_eq!(resolve("file:///x/y.pdf", &base), PathBuf::from("/x/y.pdf"));
        assert_eq!(fresh_name(&base, "cut", "mp3"), "cut.mp3");
        std::fs::write(base.join("cut.mp3"), b"x").unwrap();
        assert_eq!(fresh_name(&base, "cut", "mp3"), "cut-2.mp3");
        let out = output(&json!({"out": "deep/er/x.txt"}), "out", &base, "d.txt").unwrap();
        assert!(base.join("deep/er").is_dir() && out.ends_with("deep/er/x.txt"));
        assert!(input(&json!({"path": "nope.pdf"}), "path", &base, "t").is_err());
        assert!(
            input(&json!({}), "path", &base, "hex_x")
                .unwrap_err()
                .contains("needs “path”")
        );
        assert_eq!(seconds(&json!("1:20")), Some(80.0));
        assert_eq!(seconds(&json!(" 2.5s")), Some(2.5));
        assert_eq!(seconds(&json!(-1)), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn hidden_folders_and_library_are_private_but_icloud_is_not() {
        let home = Path::new("/Users/felix");
        for path in [
            "/Users/felix/.ssh/id_ed25519",
            "/Users/felix/.agent-browser/namespaces/hex/state/sessions/hex-sign-in.json",
            "/Users/felix/.local/share/opencode/auth.json",
            "/Users/felix/Library/Application Support/Google/Chrome/Default/Cookies",
            "/Users/felix/Library/Keychains/login.keychain-db",
            "/Users/felix/dev/app/.env",
        ] {
            assert!(private_in(Path::new(path), home), "{path}");
        }
        for path in [
            "/Users/felix/Documents/Vertrag.pdf",
            "/Users/felix/Library/Mobile Documents/com~apple~CloudDocs/Notes.pdf",
            "/Users/felix/Library/CloudStorage/Dropbox/Plan.xlsx",
            "/Volumes/Stick/photo.jpg",
        ] {
            assert!(!private_in(Path::new(path), home), "{path}");
        }
    }

    #[test]
    fn tools_write_in_their_own_folders_and_only_new_files_elsewhere() {
        let base = std::env::temp_dir().join(format!("hex-out-policy-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        // The task's own folder (inside the temp folder here): anything,
        // overwriting too.
        let own = base.join("result.png");
        std::fs::write(&own, b"old").unwrap();
        assert!(may_write(&own, &base).is_ok());
        assert!(may_write(&base.join("new/sub/video.mp4"), &base).is_ok());
        // Leaving it by `..`, or relative: never.
        assert!(may_write(&base.join("../x.png"), &base).is_err());
        assert!(may_write(Path::new("x.png"), &base).is_err());
        #[cfg(unix)]
        {
            // A link inside that points to a file elsewhere: never.
            let elsewhere =
                std::env::temp_dir().join(format!("hex-out-target-{}", std::process::id()));
            std::fs::write(&elsewhere, b"precious").unwrap();
            let link = base.join("cookie.png");
            std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
            assert!(may_write(&link, &base).is_err());
            let _ = std::fs::remove_file(&elsewhere);
        }
        // Nowhere outside the task's, Agent-Desk's and the person's folders.
        assert!(may_write(Path::new("/etc/hosts.png"), &base).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
