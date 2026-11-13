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

/// An existing file the agent named (argument `key`).
pub fn input(arguments: &Value, key: &str, base: &Path, tool: &str) -> Result<PathBuf, String> {
    let named = arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| format!("{tool} needs “{key}”: a file's path"))?;
    let path = resolve(named, base);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("There is no file “{named}”."))
    }
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
