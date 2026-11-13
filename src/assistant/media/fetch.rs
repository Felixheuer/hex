//! Getting a video from wherever it is: any page yt-dlp can read (its own
//! extractors, and its generic one for any page with a video), at most 720p,
//! with the page's own facts (title, channel, description, tags, chapters)
//! and its captions when it has them. A download is kept in Hex's video
//! cache, so asking again about the same video is instant.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::assistant::backends::{run, search};

/// The best stream up to 720p (merged into MP4 where it must be merged).
const FORMAT: &str = "bv*[height<=720]+ba/b[height<=720]/wv*+ba/w";
/// Captions to keep: the video's own language (YouTube marks it `-orig`),
/// then English and German.
const CAPTION_LANGUAGES: &str = ".*-orig,en.*,de.*";

/// What the page says about its video.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct About {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub views: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// The languages of the page's own captions, and of automatic ones.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub captions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub automatic_captions: Vec<String>,
}

/// A downloaded video.
pub struct Fetched {
    pub video: PathBuf,
    pub about: About,
    /// Its captions file, the page's own before automatic ones.
    pub captions: Option<PathBuf>,
}

/// Downloads `url` into `dir` (or finds it there from before).
pub fn fetch(
    yt_dlp: &Path,
    ffmpeg: &Path,
    url: &str,
    dir: &Path,
    limit: Duration,
) -> Result<Fetched, String> {
    if let Some(found) = cached(dir) {
        return Ok(found);
    }
    let mut command = Command::new(yt_dlp);
    command
        .args([
            "--no-playlist",
            "--no-progress",
            "--newline",
            "--no-color",
            "--no-mtime",
            "--ignore-config",
            "--socket-timeout",
            "15",
            "--match-filter",
            "!is_live",
            "--max-filesize",
            "1500M",
            "-f",
            FORMAT,
            "--merge-output-format",
            "mp4",
            "--write-info-json",
            "--write-subs",
            "--write-auto-subs",
            "--sub-langs",
            CAPTION_LANGUAGES,
            "--sub-format",
            "vtt/srt/best",
            "--ffmpeg-location",
        ])
        .arg(ffmpeg);
    // YouTube's player needs a JavaScript runtime; deno is yt-dlp's
    // default, Node (which agents have) works as well.
    if search::find("deno").is_none()
        && let Some(node) = search::find("node")
    {
        command
            .arg("--js-runtimes")
            .arg(format!("node:{}", node.display()));
    }
    command
        .arg("-o")
        .arg(dir.join("video.%(ext)s"))
        .arg("--")
        .arg(url);
    let ran =
        run::bounded(command, limit).map_err(|error| format!("couldn't start yt-dlp: {error}"))?;
    if let Some(found) = cached(dir) {
        return Ok(found);
    }
    if ran.timed_out() {
        return Err(format!(
            "the video took longer than {} s to download; try a shorter one, or a focus with a time range",
            limit.as_secs()
        ));
    }
    Err(why(&format!("{}\n{}", ran.stderr, ran.stdout)))
}

/// A download already in `dir`: the video, its facts, its captions.
fn cached(dir: &Path) -> Option<Fetched> {
    let mut video = None;
    let mut captions: Vec<PathBuf> = Vec::new();
    let mut info = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().into_owned();
        if name.ends_with(".info.json") {
            info = Some(path);
        } else if name.ends_with(".vtt") || name.ends_with(".srt") {
            captions.push(path);
        } else if name.starts_with("video.")
            && !name.ends_with(".part")
            && !name.ends_with(".ytdl")
            && !name.ends_with(".json")
            // A stream waiting to be merged (`video.f137.mp4`, `video.temp.mp4`).
            && !name.split('.').any(|part| {
                part == "temp"
                    || (part.len() > 1
                        && part.starts_with('f')
                        && part[1..].chars().all(|c| c.is_ascii_digit()))
            })
        {
            video = Some(path);
        }
    }
    let video = video?;
    let info: Value = info
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let about = about(&info);
    let captions = best_captions(&captions, &about);
    Some(Fetched {
        video,
        about,
        captions,
    })
}

/// The captions file to read: the page's own language first, then one the
/// video speaks (`-orig`), then any.
fn best_captions(files: &[PathBuf], about: &About) -> Option<PathBuf> {
    let language = |path: &PathBuf| -> String {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = name.trim_end_matches(".vtt").trim_end_matches(".srt");
        stem.rsplit_once('.')
            .map(|(_, lang)| lang.to_owned())
            .unwrap_or_default()
    };
    files
        .iter()
        .find(|file| about.captions.contains(&language(file)))
        .or_else(|| files.iter().find(|file| language(file).ends_with("-orig")))
        .or_else(|| files.first())
        .cloned()
}

/// yt-dlp's info JSON as what the page says.
pub fn about(info: &Value) -> About {
    let text = |key: &str| {
        info[key]
            .as_str()
            .map(str::to_owned)
            .filter(|text| !text.is_empty())
    };
    let languages = |key: &str| -> Vec<String> {
        let mut found: Vec<String> = info[key]
            .as_object()
            .map(|map| {
                map.keys()
                    .filter(|lang| *lang != "live_chat")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found.truncate(12);
        found
    };
    About {
        title: text("title").unwrap_or_default(),
        channel: text("channel").or_else(|| text("uploader")),
        uploaded: text("upload_date").map(|date| {
            if date.len() == 8 {
                format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
            } else {
                date
            }
        }),
        site: text("extractor_key"),
        url: text("webpage_url"),
        views: info["view_count"].as_u64(),
        likes: info["like_count"].as_u64(),
        description: text("description")
            .map(|description| crate::assistant::store::shorten(description.trim(), 2000))
            .unwrap_or_default(),
        tags: info["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(20)
            .map(str::to_owned)
            .collect(),
        chapters: info["chapters"]
            .as_array()
            .into_iter()
            .flatten()
            .take(30)
            .filter_map(|chapter| {
                Some(format!(
                    "{} {}",
                    super::frames::clock(chapter["start_time"].as_f64()?),
                    chapter["title"].as_str()?
                ))
            })
            .collect(),
        license: text("license"),
        captions: languages("subtitles"),
        automatic_captions: {
            let all = languages("automatic_captions");
            // Automatic captions come translated into every language: name
            // only the original.
            let original: Vec<String> = all
                .iter()
                .filter(|lang| lang.ends_with("-orig"))
                .cloned()
                .collect();
            if original.is_empty() {
                all.into_iter().take(3).collect()
            } else {
                original
            }
        },
    }
}

/// What went wrong, from yt-dlp's last `ERROR:` line, in plain words where
/// the cause is common.
pub fn why(output: &str) -> String {
    let error = output
        .lines()
        .rev()
        .find(|line| line.contains("ERROR:"))
        .or_else(|| output.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or("yt-dlp said nothing")
        .trim();
    let lower = output.to_lowercase();
    let plain = if lower.contains("not a bot") || lower.contains("sign in to confirm") {
        "The site wants a sign-in before it hands out this video (its bot check). "
    } else if lower.contains("unsupported url") {
        "No video was found on that page. "
    } else if lower.contains("private video") || lower.contains("members-only") {
        "The video isn't public. "
    } else if lower.contains("video unavailable") || lower.contains("http error 404") {
        "The video isn't available (removed, or not in this country). "
    } else if lower.contains("is_live") || lower.contains("does not pass filter") {
        "It's a live stream; Hex watches recordings, so try again once it has ended. "
    } else if lower.contains("larger than max-filesize") {
        "The video is too large to download here. "
    } else {
        ""
    };
    format!(
        "{plain}yt-dlp: {}",
        crate::assistant::store::shorten(error, 400)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_page_s_facts_are_kept_compact() {
        let info = json!({
            "title": "Neon city at night", "uploader": "Studio X", "upload_date": "20261020",
            "extractor_key": "Youtube", "webpage_url": "https://www.youtube.com/watch?v=abc",
            "view_count": 1200, "description": "Made with a text-to-video model.\nPrompt: …",
            "tags": ["ai", "cyberpunk"],
            "chapters": [{"start_time": 0.0, "title": "Intro"}, {"start_time": 75.0, "title": "Drone shot"}],
            "subtitles": {"en": []},
            "automatic_captions": {"en-orig": [], "de": [], "fr": [], "live_chat": []}
        });
        let about = about(&info);
        assert_eq!(about.channel.as_deref(), Some("Studio X"));
        assert_eq!(about.uploaded.as_deref(), Some("2026-10-20"));
        assert_eq!(about.chapters, ["0:00 Intro", "1:15 Drone shot"]);
        assert_eq!(about.captions, ["en"]);
        assert_eq!(about.automatic_captions, ["en-orig"]);
        assert!(about.description.starts_with("Made with"));
        let files = [
            PathBuf::from("/d/video.de.vtt"),
            PathBuf::from("/d/video.en-orig.vtt"),
            PathBuf::from("/d/video.en.vtt"),
        ];
        assert_eq!(
            best_captions(&files, &about),
            Some(PathBuf::from("/d/video.en.vtt"))
        );
        let none = About::default();
        assert_eq!(
            best_captions(&files, &none),
            Some(PathBuf::from("/d/video.en-orig.vtt"))
        );
    }

    #[test]
    fn yt_dlp_s_errors_read_plainly() {
        let bot = "[youtube] abc: Downloading webpage\nERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser";
        assert!(
            why(bot).starts_with("The site wants a sign-in"),
            "{}",
            why(bot)
        );
        assert!(why(bot).contains("ERROR: [youtube] abc"));
        assert!(
            why("ERROR: Unsupported URL: https://example.com/").starts_with("No video was found")
        );
        assert!(why("weird").ends_with("yt-dlp: weird"));
    }

    #[test]
    fn a_finished_download_is_found_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("hex-media-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(cached(&dir).is_none());
        std::fs::write(dir.join("video.f137.mp4.part"), b"").unwrap();
        assert!(cached(&dir).is_none(), "a part isn't a video");
        std::fs::write(dir.join("video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("video.info.json"), br#"{"title":"T"}"#).unwrap();
        std::fs::write(dir.join("video.en.vtt"), b"WEBVTT").unwrap();
        let found = cached(&dir).unwrap();
        assert_eq!(found.video, dir.join("video.mp4"));
        assert_eq!(found.about.title, "T");
        assert_eq!(found.captions, Some(dir.join("video.en.vtt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
