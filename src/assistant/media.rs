//! `hex_watch_video`: watch a video the way a person would, without a
//! browser that may not play it (the hidden browser's Chrome for Testing
//! has no H.264). Any video URL (YouTube, Vimeo, any page with a video,
//! through yt-dlp) or a local file: downloaded at most 720p into Hex's video
//! cache, its scenes found and shown as keyframes and one contact sheet, its
//! facts (duration, resolution, frame rate, codecs, the container's tags,
//! the page's title, channel, description, chapters), what is said (the
//! page's captions, else Hex's own transcription engine), and how it sounds
//! (speech or music, level, loudness, tempo). The frames come back as images
//! the model sees, plus their paths.
//!
//! Beside it, the media tools that make and change files, each writing
//! into the task's folder and showing images where there are any:
//! `hex_document_read` and `hex_document_write` (`document`), `hex_image`
//! (`image`), `hex_audio` (`audio`), and `hex_video_make` (`make`).
//!
//! Tools of Agent-Desk's MCP server (`tools`), for every role.
//!
//! OWNER: brain.

mod audio;
mod captions;
pub mod codecs;
pub mod document;
mod draw;
mod fetch;
mod frames;
mod image;
mod make;
pub(in crate::assistant) mod out;
mod probe;
mod sound;
mod tools;
mod zip;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

pub use tools::status;

pub const NAME: &str = "hex_watch_video";

/// Every media tool, in the order they are offered.
pub const NAMES: [&str; 6] = [
    NAME,
    document::READ,
    document::WRITE,
    image::NAME,
    audio::NAME,
    make::NAME,
];

/// The prompt's rules for them (`agent::prompt`): watching a video, and
/// one line for the rest.
pub const RULES: &str = "To judge or describe a video (on any page, in the person's tab, or a file), call hex_watch_video with its URL or path; never play it in a browser.\n\
Files and media: read any document (PDF and scans, Office, iWork) with hex_document_read and write .docx/.xlsx/.pptx/.pdf/.md with hex_document_write; inspect, crop, annotate, convert, compare, or OCR images with hex_image; transcribe, cut, mix, or speak audio into files with hex_audio; make videos (scenes, titles, voice, music, a recording of your browser) with hex_video_make.\n";

/// Every media tool's definition, in [`NAMES`]' order.
pub fn definitions() -> Vec<Value> {
    let mut all = vec![definition()];
    all.extend(document::definitions());
    all.push(image::definition());
    all.push(audio::definition());
    all.push(make::definition());
    all
}

/// Everything it may take: a three-minute video takes well under a minute.
const BUDGET: Duration = Duration::from_secs(110);
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(75);
const DEFAULT_FRAMES: usize = 24;
const DEFAULT_INLINE: usize = 8;
/// Downloads kept, at most, and for how long.
const CACHE_KEPT: usize = 12;
const CACHE_DAYS: u64 = 7;

pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "Watch a video and see it: any video URL (YouTube, Vimeo, social sites, any web page with a video, including the one in the person's open tab: get its URL with hex_browser_tab or hex_lookup) or a local video file. Returns its keyframes as images (scene changes plus evenly spaced frames, with their times and file paths), a contact sheet of all of them, metadata (duration, resolution, frame rate, codecs, the file's encoder tags; the page's title, channel, date, description, tags, chapters), the transcript (the page's captions, else transcribed locally), and how it sounds (speech, music, tempo, loudness). Use it to judge whether a video is AI-generated, how it was made, or what happens in it. Takes up to a minute or two; a second call on the same video is fast, so call again with a focus time range to look closer.",
        "inputSchema": {"type": "object", "properties": {
            "source": {"type": "string", "description": "The video's URL (http/https) or the absolute path of a video file."},
            "focus": {"type": "string", "description": "What to look for, in the person's words; a time range in it (\u{201c}1:20-1:45\u{201d}, \u{201c}at 0:30\u{201d}) puts every frame there."},
            "max_frames": {"type": "integer", "minimum": 4, "maximum": 40, "description": "Most frames taken (default 24)."},
            "inline_frames": {"type": "integer", "minimum": 0, "maximum": 24, "description": "How many of them come back as images besides the contact sheet (default 8); the rest are paths."}
        }, "required": ["source"]},
    })
}

pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// The media tool `name`'s MCP content (text, then images) for
/// `arguments`, for the task `task` (its folder gets the results);
/// `progress` hears each step.
pub fn call(
    name: &str,
    arguments: &Value,
    task: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    if name == NAME {
        return watch_call(arguments, task, progress);
    }
    let folder = out::folder(task)?;
    match name {
        document::READ => document::call_read(arguments, &folder),
        document::WRITE => document::call_write(arguments, &folder),
        image::NAME => image::call(arguments, &folder),
        audio::NAME => audio::call(arguments, &folder, progress),
        make::NAME => make::call(arguments, &folder, task, progress),
        other => Err(format!("There is no tool “{other}”.")),
    }
}

/// `hex_watch_video`.
fn watch_call(
    arguments: &Value,
    task: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    let source = arguments
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .ok_or_else(|| format!("{NAME} needs “source”: a video URL or file path"))?;
    let focus = arguments
        .get("focus")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let max_frames = arguments
        .get("max_frames")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_FRAMES, |max| (max as usize).clamp(4, 40));
    let inline = arguments
        .get("inline_frames")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_INLINE, |inline| (inline as usize).min(24));
    let watched = watch(source, focus, max_frames, task, progress)?;
    Ok(content(&watched, inline))
}

/// Where a watch's source is.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Url(String),
    File(PathBuf),
}

/// `source` as a URL or an existing file (`~/…` and `file://…` too).
pub fn source(source: &str) -> Result<Source, String> {
    let lower = source.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(Source::Url(source.to_owned()));
    }
    let path = source.strip_prefix("file://").unwrap_or(source);
    let path = match path.strip_prefix("~/") {
        Some(rest) => crate::app_paths::home_dir()
            .ok_or("no home folder")?
            .join(rest),
        None => PathBuf::from(path),
    };
    if path.is_file() {
        Ok(Source::File(path))
    } else {
        Err(format!(
            "“{source}” is neither an http(s) URL nor a video file on this Mac"
        ))
    }
}

/// What watching found.
struct Watched {
    report: Value,
    frames: Vec<frames::Frame>,
    sheet: Option<PathBuf>,
}

fn watch(
    raw: &str,
    focus: &str,
    max_frames: usize,
    task: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<Watched, String> {
    let started = Instant::now();
    let deadline = started + BUDGET;
    let source = source(raw)?;
    let key = tools::sha256_hex(raw.as_bytes())[..16].to_owned();
    let cache = cache_root()?;
    prune(&cache, &key);
    let work = cache.join(&key);
    crate::app_paths::create_private_dir(&work).map_err(|error| error.to_string())?;
    let ffmpeg = tools::ffmpeg(progress)?;
    let mut took = serde_json::Map::new();
    let mut mark = |name: &str, since: Instant| {
        took.insert(
            name.into(),
            json!((since.elapsed().as_secs_f64() * 10.0).round() / 10.0),
        );
    };

    // The video, and what its page says.
    let step = Instant::now();
    let (video, about, caption_file) = match &source {
        Source::Url(url) => {
            progress(&format!("Downloading the video from {}", host(url)));
            let yt_dlp = tools::yt_dlp(progress)?;
            let fetched = fetch::fetch(&yt_dlp, &ffmpeg, url, &work, DOWNLOAD_LIMIT)?;
            (fetched.video, Some(fetched.about), fetched.captions)
        }
        Source::File(path) => (path.clone(), None, None),
    };
    mark("download", step);
    let meta = probe::probe(&ffmpeg, &video)?;

    // Its pictures.
    let window = frames::window_in(focus, meta.duration_s);
    let out = frames_dir(task, &work, &key, window)?;
    let step = Instant::now();
    progress("Finding the video's scenes");
    let scan_limit = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(30));
    let scenes = frames::scene_changes(&ffmpeg, &video, meta.duration_s, window, scan_limit)
        .unwrap_or_default();
    mark("scenes", step);
    let moments = frames::choose(&scenes, meta.duration_s, window, max_frames);
    let step = Instant::now();
    progress(&format!("Taking {} frames", moments.len()));
    let shots = frames::extract(&ffmpeg, &video, &moments, &out, deadline);
    if shots.is_empty() {
        return Err("ffmpeg couldn't take any frame of the video".into());
    }
    let sheet = frames::contact_sheet(&ffmpeg, &shots, &out);
    mark("frames", step);

    // Its sound and words.
    let step = Instant::now();
    let mut transcript = json!({"from": "none"});
    let mut said_words = None;
    let mut heard = None;
    if meta.audio.is_some() {
        progress("Listening to the sound");
        let limit = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(30));
        match sound::extract(&ffmpeg, &video, &work, limit) {
            Ok(sound) => heard = Some(sound),
            Err(error) => transcript = json!({"from": "none", "note": error}),
        }
    }
    let caption_lines = caption_file
        .as_ref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map(|text| captions::parse(&text))
        .filter(|lines| !lines.is_empty());
    let lines = match (caption_lines, &heard) {
        (Some(lines), _) => {
            let language = caption_file
                .as_ref()
                .and_then(|file| file.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .and_then(|stem| stem.rsplit_once('.').map(|(_, lang)| lang.to_owned()))
                .unwrap_or_default();
            transcript = json!({"from": format!("the page's captions ({language})")});
            Some((lines, false))
        }
        (None, Some(sound)) if !sound.samples.is_empty() => {
            progress("Transcribing what is said");
            match sound::transcribe(&sound.samples, deadline) {
                Ok(done) => {
                    transcript = json!({"from": "Hex's transcription"});
                    Some(done)
                }
                Err(error) => {
                    transcript = json!({"from": "none", "note": error});
                    None
                }
            }
        }
        _ => None,
    };
    if let Some((lines, out_of_time)) = lines {
        said_words = Some(
            lines
                .iter()
                .map(|line| line.text.split_whitespace().count())
                .sum::<usize>(),
        );
        let (text, cut) = captions::paragraphs(&lines, 30.0, 12_000);
        transcript["text"] = json!(if text.is_empty() {
            "(nothing said)".to_owned()
        } else {
            text
        });
        if cut || out_of_time {
            transcript["cut"] = json!(true);
        }
    }
    let character = heard.as_ref().map(|sound| {
        let mut character = sound::character(&sound.samples, said_words);
        character.loudness_lufs = sound.lufs;
        character.loudness_range_lu = sound.loudness_range;
        character
    });
    mark("sound", step);
    mark("total", started);

    let frame_list: Vec<Value> = shots
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            json!({"n": index + 1, "at": frame.clock, "at_s": frame.at_s,
                "scene_change": frame.scene_change, "path": frame.path})
        })
        .collect();
    let mut report = json!({
        "source": raw,
        "video": {
            "duration": frames::clock(meta.duration_s),
            "duration_s": (meta.duration_s * 10.0).round() / 10.0,
            "width": meta.width, "height": meta.height, "fps": meta.fps,
            "codec": meta.video_codec, "bitrate_kbps": meta.bitrate_kbps,
            "audio": meta.audio, "file_tags": meta.tags,
            "file": video,
        },
        "frames": frame_list,
        "scene_changes_found": scenes.len(),
        "sound": character,
        "transcript": transcript,
        "took_s": took,
    });
    if let Some(about) = about {
        report["page"] = json!(about);
    }
    if let Some((start, end)) = window {
        report["focus_window"] = json!(format!("{}-{}", frames::clock(start), frames::clock(end)));
    }
    if let Some(sheet) = &sheet {
        report["contact_sheet"] =
            json!({"path": sheet, "order": "frames 1.. left to right, top to bottom"});
    }
    Ok(Watched {
        report,
        frames: shots,
        sheet,
    })
}

/// The report as text, then the contact sheet and up to `inline` frames,
/// spread over the video, as images.
fn content(watched: &Watched, inline: usize) -> Vec<Value> {
    let shown = spread(watched.frames.len(), inline);
    let mut report = watched.report.clone();
    report["frames_shown_as_images"] =
        json!(shown.iter().map(|index| index + 1).collect::<Vec<_>>());
    report["note"] = json!(
        "The images follow: the contact sheet first (if any), then the frames listed in frames_shown_as_images. Read a frame's path to see one that isn't shown; call again with a focus time range to look closer at a moment."
    );
    let mut content = vec![json!({"type": "text", "text": report.to_string()})];
    let images = watched
        .sheet
        .iter()
        .chain(shown.iter().map(|index| &watched.frames[*index].path));
    for path in images {
        if let Ok(bytes) = std::fs::read(path) {
            content.push(json!({"type": "image", "mimeType": "image/jpeg",
                "data": base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
    }
    content
}

/// `count` indexes of `total`, evenly spread, first and last included.
fn spread(total: usize, count: usize) -> Vec<usize> {
    if count == 0 || total == 0 {
        return Vec::new();
    }
    if count >= total {
        return (0..total).collect();
    }
    if count == 1 {
        return vec![0];
    }
    let mut picked: Vec<usize> = (0..count)
        .map(|index| (index * (total - 1) + (count - 1) / 2) / (count - 1))
        .collect();
    picked.dedup();
    picked
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_owned())
        })
        .unwrap_or_else(|| "the web".into())
}

fn cache_root() -> Result<PathBuf, String> {
    let root = crate::assistant::store::assistant_dir()
        .map_err(|error| format!("{error:#}"))?
        .join("video-cache");
    crate::app_paths::create_private_dir(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

/// Where the frames go: the task's own folder when it has one (so they stay
/// with its work), else beside the download.
fn frames_dir(
    task: Option<&str>,
    work: &Path,
    key: &str,
    window: Option<(f64, f64)>,
) -> Result<PathBuf, String> {
    let name = match window {
        Some((start, end)) => format!("video-{}-{start:.0}-{end:.0}", &key[..8]),
        None => format!("video-{}", &key[..8]),
    };
    let folder = task
        .and_then(|task| {
            crate::assistant::store::Store::open()
                .ok()?
                .load(task)?
                .workspace
        })
        .filter(|folder| folder.is_dir())
        .map_or_else(|| work.join(&name), |folder| folder.join(&name));
    let _ = std::fs::remove_dir_all(&folder);
    crate::app_paths::create_private_dir(&folder).map_err(|error| error.to_string())?;
    Ok(folder)
}

/// Keeps the cache small: downloads older than [`CACHE_DAYS`], and all but
/// the newest [`CACHE_KEPT`], go (never `keep`).
fn prune(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_name() != keep)
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    dirs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let old = std::time::SystemTime::now() - Duration::from_secs(CACHE_DAYS * 86_400);
    for (index, (modified, path)) in dirs.into_iter().enumerate() {
        if index + 1 >= CACHE_KEPT || modified < old {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// The overlay's line for a media call (`provider::desktop`): `name`
/// without its `hex_`.
pub fn step(name: &str, arguments: &Value) -> Option<(String, String)> {
    let file = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .and_then(|path| Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned())
    };
    let pair = |doing: String, done: String| Some((doing, done));
    match name {
        "watch_video" => Some(watch_step(arguments)),
        "document_read" => {
            let what = file("path").unwrap_or_else(|| "a document".into());
            pair(format!("Reading {what}"), format!("Read {what}"))
        }
        "document_write" => {
            let what = file("path").unwrap_or_else(|| "a document".into());
            pair(format!("Writing {what}"), format!("Wrote {what}"))
        }
        "image" => {
            let what = file("path").unwrap_or_else(|| "an image".into());
            pair(format!("Working on {what}"), format!("Worked on {what}"))
        }
        "audio" => match arguments.get("op").and_then(Value::as_str) {
            Some("transcribe") => {
                let what = file("path").unwrap_or_else(|| "a recording".into());
                pair(
                    format!("Transcribing {what}"),
                    format!("Transcribed {what}"),
                )
            }
            Some("speak") => pair("Recording a voice".into(), "Recorded a voice".into()),
            _ => pair("Editing sound".into(), "Edited sound".into()),
        },
        "video_make" => match arguments.get("record").and_then(Value::as_str) {
            Some("start") => pair(
                "Recording the browser".into(),
                "Recording the browser".into(),
            ),
            Some(_) => pair("Saving the recording".into(), "Saved the recording".into()),
            None => pair("Making a video".into(), "Made a video".into()),
        },
        _ => None,
    }
}

/// The overlay's line for watching: what is watched.
fn watch_step(arguments: &Value) -> (String, String) {
    let what = match arguments.get("source").and_then(Value::as_str).map(source) {
        Some(Ok(Source::Url(url))) => format!("the video from {}", host(&url)),
        Some(Ok(Source::File(path))) => path.file_name().map_or_else(
            || "the video".into(),
            |name| name.to_string_lossy().into_owned(),
        ),
        _ => "the video".into(),
    };
    (format!("Watching {what}"), format!("Watched {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_urls_or_files_and_frames_spread_evenly() {
        assert_eq!(
            source("https://www.youtube.com/watch?v=abc"),
            Ok(Source::Url("https://www.youtube.com/watch?v=abc".into()))
        );
        assert!(source("/no/such/video.mp4").is_err());
        assert!(source("").is_err());
        let file = std::env::temp_dir().join(format!("hex-media-src-{}.mp4", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            source(&format!("file://{}", file.display())),
            Ok(Source::File(file.clone()))
        );
        let (doing, done) =
            step("watch_video", &json!({"source": file.to_string_lossy()})).unwrap();
        assert!(doing.starts_with("Watching hex-media-src-") && done.starts_with("Watched"));
        let _ = std::fs::remove_file(&file);
        assert_eq!(
            step("watch_video", &json!({"source": "https://vimeo.com/1"}))
                .unwrap()
                .0,
            "Watching the video from vimeo.com"
        );
        assert_eq!(
            step("document_write", &json!({"path": "/w/Bericht.docx"}))
                .unwrap()
                .1,
            "Wrote Bericht.docx"
        );
        assert!(step("lookup", &json!({})).is_none());
        assert_eq!(spread(24, 8), [0, 3, 7, 10, 13, 16, 20, 23]);
        assert_eq!(spread(3, 8), [0, 1, 2]);
        assert_eq!(spread(10, 1), [0]);
        assert!(spread(10, 0).is_empty());
        assert!(
            call(NAME, &json!({}), None, &mut |_| {})
                .unwrap_err()
                .contains("needs “source”")
        );
        assert_eq!(definition()["name"], NAME);
        assert!(handles(NAME) && handles("hex_image") && !handles("hex_lookup"));
        let names: Vec<Value> = definitions()
            .iter()
            .map(|tool| tool["name"].clone())
            .collect();
        assert_eq!(names, NAMES.map(|name| json!(name)));
        for name in NAMES {
            assert!(RULES.contains(name), "the rules name {name}");
        }
    }

    /// Watches HEX_MEDIA_SOURCE (a file or URL) and prints the report:
    /// `HEX_MEDIA_SOURCE=… cargo test --bin hex media::tests::watch_a_given_source -- --ignored --nocapture`.
    #[test]
    #[ignore = "watches a real video"]
    fn watch_a_given_source() {
        let source = std::env::var("HEX_MEDIA_SOURCE").expect("HEX_MEDIA_SOURCE");
        let started = Instant::now();
        let content = call(NAME, &json!({"source": source}), None, &mut |step| {
            println!("{:5.1} s  {step}", started.elapsed().as_secs_f64());
        })
        .unwrap();
        println!("took {:.1} s", started.elapsed().as_secs_f64());
        println!("{}", content[0]["text"].as_str().unwrap());
        println!(
            "{} images",
            content
                .iter()
                .filter(|item| item["type"] == "image")
                .count()
        );
    }

    /// Watches a short test video made here with ffmpeg (no network):
    /// `cargo test --bin hex media::tests::watch_a_made_video -- --ignored --nocapture`.
    #[test]
    #[ignore = "runs ffmpeg"]
    fn watch_a_made_video() {
        let ffmpeg = tools::ffmpeg(&mut |_| {}).expect("ffmpeg");
        let dir = std::env::temp_dir().join(format!("hex-media-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("test.mp4");
        let made = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=20:size=1280x720:rate=30",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "smptebars=duration=20:size=1280x720:rate=30",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=40"])
            .args([
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0[v]",
                "-map",
                "[v]",
                "-map",
                "2:a",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&video)
            .status()
            .unwrap();
        assert!(made.success());
        let started = Instant::now();
        let mut steps = Vec::new();
        let content = call(
            NAME,
            &json!({"source": video.to_string_lossy()}),
            None,
            &mut |step| steps.push(step.to_owned()),
        )
        .unwrap();
        println!(
            "took {:.1} s, steps {steps:?}",
            started.elapsed().as_secs_f64()
        );
        println!("{}", content[0]["text"].as_str().unwrap());
        let report: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(report["video"]["width"], 1280);
        assert!(report["frames"].as_array().unwrap().len() >= 4);
        assert!(
            report["scene_changes_found"].as_u64().unwrap() >= 1,
            "the cut at 0:20"
        );
        assert!(
            content
                .iter()
                .filter(|item| item["type"] == "image")
                .count()
                >= 2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
