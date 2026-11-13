//! `hex_video_make`: make a video. Scenes from pictures, screenshots,
//! clips, or plain colors, each with an optional title or caption (drawn
//! here, `draw`) and a line spoken by Agent-Desk's voice; music under it
//! all; cut and encoded by ffmpeg into an MP4. It also records the task's
//! hidden browser (agent-browser's `record`) for a demo of a website, and
//! renders a Remotion project when Node is there.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::draw::{self, Canvas};
use super::{audio, out, tools};

pub const NAME: &str = "hex_video_make";

/// How long making a video may take, and one ffmpeg step.
const BUDGET: Duration = Duration::from_secs(540);
const STEP_LIMIT: Duration = Duration::from_secs(240);
const MAX_SCENES: usize = 60;
const MAX_SCENE_SECONDS: f64 = 120.0;
const DEFAULT_SECONDS: f64 = 4.0;
/// Frames shown of the result.
const SHOWN: usize = 6;

pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "Make an MP4 video: scenes from images or screenshots, video clips, or a color, each with a title or caption and a line spoken by Agent-Desk's voice (say), music underneath, fades between; or record your hidden browser for a website demo (record \"start\" with an optional url, drive the browser, then record \"stop\"); or render a Remotion project (Node required). Returns the file and frames of the result to check it.",
        "inputSchema": {"type": "object", "properties": {
            "out": {"type": "string", "description": "The video file (default video.mp4 in your folder)."},
            "scenes": {"type": "array", "items": {"type": "object"}, "description": "[{\"image\" | \"video\" | \"color\", \"duration\" (s, default 4; a spoken line lengthens it), \"start\" (s into a clip), \"title\" (big centered text), \"text\" (a caption at the bottom), \"say\" (spoken over the scene), \"fit\": \"contain\"|\"cover\", \"zoom\": true (slow push-in on a picture)}]"},
            "size": {"type": "string", "description": "WIDTHxHEIGHT, default 1920x1080 (1080x1920 for vertical)."},
            "fps": {"type": "integer", "minimum": 10, "maximum": 60},
            "voice": {"type": "string", "description": "The voice for say: F1-F5 or M1-M5 (default M1)."},
            "language": {"type": "string", "description": "The language of the spoken lines, e.g. de (default: detected)."},
            "voiceover": {"type": "string", "description": "A sound file spoken over the whole video."},
            "music": {"type": "string", "description": "A music file under the video (looped, faded out)."},
            "music_volume": {"type": "number", "description": "0-1, default 0.25 (0.12 under speech)."},
            "fade": {"type": "number", "description": "Seconds of fade in and out per scene (default 0.3, 0 for hard cuts)."},
            "record": {"type": "string", "enum": ["start", "stop"], "description": "Record your hidden browser to a video: start, act in the browser, then stop."},
            "url": {"type": "string", "description": "record start: open this page first."},
            "session": {"type": "string", "description": "record: the browser session's name (default your main one)."},
            "remotion": {"type": "object", "description": "{\"project\": folder, \"composition\": id, \"props\": {…}}: render with Remotion (npx)."}
        }},
    })
}

/// One scene as asked.
#[derive(Clone, Debug, PartialEq)]
struct Scene {
    source: Source,
    duration: f64,
    start: f64,
    title: String,
    text: String,
    say: String,
    cover: bool,
    zoom: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Source {
    Image(PathBuf),
    Video(PathBuf),
    Color(String),
}

fn size(arguments: &Value) -> Result<(u32, u32), String> {
    let Some(text) = arguments["size"].as_str() else {
        return Ok((1920, 1080));
    };
    let (width, height) = text
        .split_once(['x', 'X', '×'])
        .and_then(|(width, height)| {
            Some((
                width.trim().parse::<u32>().ok()?,
                height.trim().parse::<u32>().ok()?,
            ))
        })
        .ok_or_else(|| format!("“{text}” isn't a size like 1920x1080"))?;
    if !(64..=3840).contains(&width) || !(64..=3840).contains(&height) {
        return Err("the size must be between 64 and 3840 pixels a side".into());
    }
    // H.264 wants even sides.
    Ok((width & !1, height & !1))
}

fn scenes(arguments: &Value, folder: &Path) -> Result<Vec<Scene>, String> {
    let list = arguments["scenes"]
        .as_array()
        .filter(|list| !list.is_empty())
        .ok_or("hex_video_make needs scenes (or record, or remotion)")?;
    if list.len() > MAX_SCENES {
        return Err(format!("at most {MAX_SCENES} scenes"));
    }
    list.iter()
        .enumerate()
        .map(|(index, scene)| {
            let file = |key: &str| -> Result<Option<PathBuf>, String> {
                match scene[key].as_str() {
                    Some(path) => {
                        let resolved = out::resolve(path, folder);
                        if resolved.is_file() {
                            Ok(Some(resolved))
                        } else {
                            Err(format!("scene {}: there is no file “{path}”", index + 1))
                        }
                    }
                    None => Ok(None),
                }
            };
            let source = match (file("image")?, file("video")?) {
                (Some(image), _) => Source::Image(image),
                (None, Some(video)) => Source::Video(video),
                (None, None) => {
                    let color = scene["color"].as_str().unwrap_or("#111827");
                    let rgba = draw::color(color)
                        .ok_or_else(|| format!("scene {}: “{color}” isn't a color", index + 1))?;
                    Source::Color(format!("0x{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]))
                }
            };
            Ok(Scene {
                source,
                duration: out::seconds(&scene["duration"])
                    .unwrap_or(0.0)
                    .min(MAX_SCENE_SECONDS),
                start: out::seconds(&scene["start"]).unwrap_or(0.0),
                title: scene["title"].as_str().unwrap_or_default().to_owned(),
                text: scene["text"].as_str().unwrap_or_default().to_owned(),
                say: scene["say"].as_str().unwrap_or_default().to_owned(),
                cover: scene["fit"].as_str() == Some("cover"),
                zoom: scene["zoom"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// The scene's title and caption drawn on a transparent frame.
fn overlay(scene: &Scene, (width, height): (u32, u32), path: &Path) -> Result<bool, String> {
    if scene.title.is_empty() && scene.text.is_empty() {
        return Ok(false);
    }
    let mut canvas = Canvas::new(width, height)?;
    let (w, h) = (width as f32, height as f32);
    let short = w.min(h);
    if !scene.title.is_empty() {
        let size = short / 9.0;
        let (text_width, text_height) = draw::measure(&scene.title, size, true, w * 0.84)?;
        let (x, y) = ((w - text_width) / 2.0, (h - text_height) / 2.0);
        let pad = size * 0.45;
        canvas.fill(
            x - pad,
            y - pad,
            text_width + 2.0 * pad,
            text_height + 2.0 * pad,
            [0, 0, 0, 120],
        );
        canvas.text(
            &scene.title,
            (x, y),
            size,
            [255, 255, 255, 255],
            true,
            text_width + 1.0,
        )?;
    }
    if !scene.text.is_empty() {
        let size = short / 20.0;
        let (text_width, text_height) = draw::measure(&scene.text, size, false, w * 0.86)?;
        let pad = size * 0.5;
        let y = h - text_height - pad * 2.0 - h * 0.06;
        let x = (w - text_width) / 2.0;
        canvas.fill(
            x - pad,
            y - pad,
            text_width + 2.0 * pad,
            text_height + 2.0 * pad,
            [0, 0, 0, 160],
        );
        canvas.text(
            &scene.text,
            (x, y),
            size,
            [255, 255, 255, 255],
            false,
            text_width + 1.0,
        )?;
    }
    canvas
        .into_image()
        .save(path)
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// The ffmpeg filter that fits a picture or clip into the frame.
fn fit(scene: &Scene, (width, height): (u32, u32), fps: u32, frames: u64) -> String {
    let base = match (&scene.source, scene.zoom) {
        (Source::Image(_), true) => format!(
            "scale={w2}:{h2}:force_original_aspect_ratio=increase,crop={w2}:{h2},zoompan=z='min(1+0.12*on/{frames},1.12)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d={frames}:s={width}x{height}:fps={fps}",
            w2 = width * 2,
            h2 = height * 2
        ),
        _ if scene.cover => format!(
            "scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"
        ),
        _ => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
    };
    format!("{base},setsar=1,fps={fps},format=yuv420p")
}

/// One scene encoded as a silent segment.
fn segment(
    ffmpeg: &Path,
    scene: &Scene,
    frame: (u32, u32),
    fps: u32,
    fade: f64,
    overlay: Option<&Path>,
    target: &Path,
) -> Result<(), String> {
    let frames = (scene.duration * f64::from(fps)).round().max(1.0) as u64;
    let mut command = out::ffmpeg_command(ffmpeg);
    match &scene.source {
        Source::Image(path) if scene.zoom => {
            command.arg("-i").arg(path);
        }
        Source::Image(path) => {
            command
                .args([
                    "-loop",
                    "1",
                    "-framerate",
                    &fps.to_string(),
                    "-t",
                    &format!("{:.3}", scene.duration),
                    "-i",
                ])
                .arg(path);
        }
        Source::Video(path) => {
            command
                .args([
                    "-ss",
                    &format!("{:.3}", scene.start),
                    "-t",
                    &format!("{:.3}", scene.duration),
                    "-i",
                ])
                .arg(path);
        }
        Source::Color(color) => {
            command.args([
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c={color}:s={}x{}:r={fps}:d={:.3}",
                    frame.0, frame.1, scene.duration
                ),
            ]);
        }
    }
    let mut filter = format!("[0:v]{}", fit(scene, frame, fps, frames));
    // A clip shorter than the scene holds its last frame.
    filter.push_str(&format!(
        ",tpad=stop_mode=clone:stop_duration={:.3}",
        scene.duration
    ));
    if let Some(overlay) = overlay {
        command.args(["-loop", "1", "-i"]).arg(overlay);
        filter.push_str("[base];[base][1:v]overlay=0:0:format=auto,format=yuv420p");
    }
    if fade > 0.0 && scene.duration > fade * 2.5 {
        filter.push_str(&format!(
            ",fade=t=in:st=0:d={fade:.2},fade=t=out:st={:.3}:d={fade:.2}",
            scene.duration - fade
        ));
    }
    filter.push_str("[v]");
    command
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "[v]",
            "-an",
            "-frames:v",
            &frames.to_string(),
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-r",
            &fps.to_string(),
        ])
        .arg(target);
    out::run(command, STEP_LIMIT, "ffmpeg").map(|_| ())
}

/// The scene's own clip length from `start`.
fn clip_length(ffmpeg: &Path, scene: &Scene) -> Option<f64> {
    match &scene.source {
        Source::Video(path) => {
            audio::duration(ffmpeg, path).map(|length| (length - scene.start).max(0.5))
        }
        _ => None,
    }
}

pub fn call(
    arguments: &Value,
    folder: &Path,
    task: Option<&str>,
    progress: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    if let Some(action) = arguments["record"].as_str() {
        return record(action, arguments, folder, task);
    }
    if arguments["remotion"].is_object() {
        return remotion(&arguments["remotion"], arguments, folder);
    }
    let started = Instant::now();
    let deadline = started + BUDGET;
    let ffmpeg = tools::ffmpeg(progress)?;
    let frame = size(arguments)?;
    let fps = arguments["fps"]
        .as_u64()
        .map_or(30, |fps| fps.clamp(10, 60)) as u32;
    let fade = arguments["fade"].as_f64().unwrap_or(0.3).clamp(0.0, 2.0);
    let mut scenes = scenes(arguments, folder)?;
    let target = out::output(
        arguments,
        "out",
        folder,
        &out::fresh_name(folder, "video", "mp4"),
    )?;
    let work = folder.join(format!(".video-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let result = (|| -> Result<Vec<Value>, String> {
        let mut took = serde_json::Map::new();
        // The spoken lines first: they set how long their scenes last.
        let step = Instant::now();
        let voice = arguments["voice"].as_str().unwrap_or("M1");
        let mut spoken: Vec<Option<(PathBuf, f64)>> = Vec::new();
        let mut voices = Vec::new();
        for (index, scene) in scenes.iter_mut().enumerate() {
            if scene.say.trim().is_empty() {
                spoken.push(None);
                continue;
            }
            progress(&format!("Speaking scene {}", index + 1));
            let file = work.join(format!("say-{index}.wav"));
            let language = arguments["language"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| crate::assistant::speech::language_of(&scene.say).map(str::to_owned))
                .unwrap_or_else(crate::assistant::speech::system_language);
            voices.push(audio::speak(&scene.say, voice, &language, &file, &ffmpeg)?);
            let length = audio::duration(&ffmpeg, &file).unwrap_or(0.0);
            spoken.push(Some((file, length)));
        }
        for (scene, said) in scenes.iter_mut().zip(&spoken) {
            let own = clip_length(&ffmpeg, scene);
            let asked = if scene.duration > 0.0 {
                scene.duration
            } else {
                own.unwrap_or(DEFAULT_SECONDS).min(MAX_SCENE_SECONDS)
            };
            let needed = said.as_ref().map_or(0.0, |(_, length)| length + 0.7);
            scene.duration = asked.max(needed).max(0.5);
        }
        took.insert("voice".into(), json!(out::took(step)));

        // Each scene, then all of them in a row.
        let step = Instant::now();
        let mut list = String::new();
        for (index, scene) in scenes.iter().enumerate() {
            if Instant::now() > deadline {
                return Err(format!(
                    "making the video took too long (stopped at scene {})",
                    index + 1
                ));
            }
            progress(&format!("Making scene {} of {}", index + 1, scenes.len()));
            let overlay_path = work.join(format!("overlay-{index}.png"));
            let has_overlay = overlay(scene, frame, &overlay_path)?;
            let piece = work.join(format!("scene-{index}.mp4"));
            segment(
                &ffmpeg,
                scene,
                frame,
                fps,
                fade,
                has_overlay.then_some(overlay_path.as_path()),
                &piece,
            )?;
            list.push_str(&format!(
                "file '{}'\n",
                piece.to_string_lossy().replace('\'', "'\\''")
            ));
        }
        let list_file = work.join("scenes.txt");
        std::fs::write(&list_file, list).map_err(|error| error.to_string())?;
        let silent = work.join("silent.mp4");
        let mut command = out::ffmpeg_command(&ffmpeg);
        command
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list_file)
            .args(["-c", "copy"])
            .arg(&silent);
        out::run(command, STEP_LIMIT, "ffmpeg")?;
        took.insert("scenes".into(), json!(out::took(step)));

        // The sound: spoken lines at their scenes, a voiceover, music.
        let step = Instant::now();
        progress("Adding the sound");
        let total: f64 = scenes.iter().map(|scene| scene.duration).sum();
        let mut command = out::ffmpeg_command(&ffmpeg);
        command.arg("-i").arg(&silent);
        let mut inputs = 1;
        let mut filter = String::new();
        let mut speech = Vec::new();
        let mut at = 0.0;
        for (scene, said) in scenes.iter().zip(&spoken) {
            if let Some((file, _)) = said {
                command.arg("-i").arg(file);
                let delay = ((at + 0.35) * 1000.0) as u64;
                filter.push_str(&format!("[{inputs}:a]aresample=48000,aformat=channel_layouts=stereo,adelay={delay}:all=1[s{inputs}];"));
                speech.push(format!("[s{inputs}]"));
                inputs += 1;
            }
            at += scene.duration;
        }
        if let Some(voiceover) = arguments["voiceover"].as_str() {
            let file = out::resolve(voiceover, folder);
            if !file.is_file() {
                return Err(format!("There is no file “{voiceover}”."));
            }
            command.arg("-i").arg(&file);
            filter.push_str(&format!(
                "[{inputs}:a]aresample=48000,aformat=channel_layouts=stereo[s{inputs}];"
            ));
            speech.push(format!("[s{inputs}]"));
            inputs += 1;
        }
        let mut layers = Vec::new();
        if !speech.is_empty() {
            filter.push_str(&format!(
                "{}amix=inputs={}:duration=longest:normalize=0[speech];",
                speech.concat(),
                speech.len()
            ));
            layers.push("[speech]".to_owned());
        }
        if let Some(music) = arguments["music"].as_str() {
            let file = out::resolve(music, folder);
            if !file.is_file() {
                return Err(format!("There is no file “{music}”."));
            }
            command.args(["-stream_loop", "-1", "-i"]).arg(&file);
            let volume = arguments["music_volume"]
                .as_f64()
                .unwrap_or(if speech.is_empty() { 0.25 } else { 0.12 })
                .clamp(0.0, 1.0);
            let fade_out = (total - 2.0).max(0.0);
            filter.push_str(&format!(
                "[{inputs}:a]aresample=48000,aformat=channel_layouts=stereo,atrim=0:{total:.3},volume={volume},afade=t=in:d=1,afade=t=out:st={fade_out:.3}:d=2[music];"
            ));
            layers.push("[music]".to_owned());
        }
        if layers.is_empty() {
            std::fs::rename(&silent, &target)
                .or_else(|_| std::fs::copy(&silent, &target).map(|_| ()))
                .map_err(|error| error.to_string())?;
        } else {
            filter.push_str(&format!(
                "{}amix=inputs={}:duration=longest:normalize=0,apad[a]",
                layers.concat(),
                layers.len()
            ));
            command
                .args([
                    "-filter_complex",
                    &filter,
                    "-map",
                    "0:v",
                    "-map",
                    "[a]",
                    "-c:v",
                    "copy",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                ])
                .args(["-t", &format!("{total:.3}"), "-movflags", "+faststart"])
                .arg(&target);
            out::run(command, STEP_LIMIT, "ffmpeg")?;
        }
        took.insert("sound".into(), json!(out::took(step)));
        took.insert("total".into(), json!(out::took(started)));

        let frames = sample_frames(&ffmpeg, &target, &scenes, folder);
        let mut report = json!({
            "file": target,
            "duration_s": audio::duration(&ffmpeg, &target).map(|seconds| (seconds * 10.0).round() / 10.0),
            "size": [frame.0, frame.1],
            "fps": fps,
            "scenes": scenes.iter().map(|scene| (scene.duration * 10.0).round() / 10.0).collect::<Vec<_>>(),
            "bytes": std::fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0),
            "took_s": took,
            "frames": frames,
            "note": "The images are frames from the middle of the first scenes, to check the result.",
        });
        if !voices.is_empty() {
            report["voice"] = json!(voices[0]);
        }
        Ok(out::content(&report, &frames, SHOWN))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// A frame from the middle of each of the first scenes, as JPEGs.
fn sample_frames(ffmpeg: &Path, video: &Path, scenes: &[Scene], folder: &Path) -> Vec<PathBuf> {
    let stem = video.file_stem().map_or_else(
        || "video".into(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let mut at = 0.0;
    let mut frames = Vec::new();
    for (index, scene) in scenes.iter().enumerate() {
        let middle = at + scene.duration / 2.0;
        at += scene.duration;
        if index >= SHOWN {
            break;
        }
        let file = folder.join(out::fresh_name(
            folder,
            &format!("{stem}-frame-{}", index + 1),
            "jpg",
        ));
        let mut command = out::ffmpeg_command(ffmpeg);
        command
            .args(["-ss", &format!("{middle:.3}"), "-i"])
            .arg(video)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale='min(960,iw)':-2",
                "-q:v",
                "4",
            ])
            .arg(&file);
        if out::run(command, Duration::from_secs(30), "ffmpeg").is_ok() && file.is_file() {
            frames.push(file);
        }
    }
    frames
}

/// The task's hidden browser recorded: `start` (with an optional url) and
/// `stop`, through agent-browser's `record`.
fn record(
    action: &str,
    arguments: &Value,
    folder: &Path,
    task: Option<&str>,
) -> Result<Vec<Value>, String> {
    let task = task.ok_or("recording needs the task's browser")?;
    let path = crate::assistant::browser::executable().ok_or("agent-browser isn't installed")?;
    let session = crate::assistant::browser::sessions::full_name(
        task,
        arguments["session"].as_str().unwrap_or_default(),
    );
    let state = folder.join(".recording.json");
    match action {
        "start" => {
            let target = out::output(
                arguments,
                "out",
                folder,
                &out::fresh_name(folder, "browser-demo", "mp4"),
            )?;
            let args = record_args(
                &target,
                arguments["url"].as_str(),
                arguments["fps"].as_u64(),
            );
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            crate::assistant::browser::call(&path, &session, &refs)?;
            std::fs::write(
                &state,
                json!({"file": target, "session": session}).to_string(),
            )
            .map_err(|error| error.to_string())?;
            Ok(vec![json!({"type": "text", "text": format!(
                "Recording the browser (session {session}) to {}. Act in the browser now (open pages, scroll, click, type; wait a moment where the viewer should look), then call {NAME} with record \"stop\".",
                target.display()
            )})])
        }
        "stop" => {
            let started: Value = std::fs::read_to_string(&state)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            let session = started["session"].as_str().map_or(session, str::to_owned);
            crate::assistant::browser::call(&path, &session, &["record", "stop"])?;
            let _ = std::fs::remove_file(&state);
            let file = started["file"]
                .as_str()
                .map(PathBuf::from)
                .filter(|file| file.is_file())
                .ok_or("the recording wasn't saved")?;
            let ffmpeg = tools::ffmpeg(&mut |_| {})?;
            let length = audio::duration(&ffmpeg, &file).unwrap_or(0.0);
            let scene = Scene {
                source: Source::Video(file.clone()),
                duration: length,
                start: 0.0,
                title: String::new(),
                text: String::new(),
                say: String::new(),
                cover: false,
                zoom: false,
            };
            let frames = sample_frames(&ffmpeg, &file, std::slice::from_ref(&scene), folder);
            let report = json!({"file": file, "duration_s": (length * 10.0).round() / 10.0,
                "next": "Use the file as a scene's video in hex_video_make to add titles, a voice, and music."});
            Ok(out::content(&report, &frames, 1))
        }
        other => Err(format!("record is start or stop, not “{other}”")),
    }
}

/// agent-browser's `record start` arguments.
fn record_args(target: &Path, url: Option<&str>, fps: Option<u64>) -> Vec<String> {
    let mut args = vec![
        "record".to_owned(),
        "start".to_owned(),
        target.to_string_lossy().into_owned(),
    ];
    if let Some(url) = url.filter(|url| url.starts_with("http://") || url.starts_with("https://")) {
        args.push(url.to_owned());
    }
    args.extend([
        "--fps".to_owned(),
        fps.unwrap_or(30).clamp(1, 60).to_string(),
        "--cursor".to_owned(),
    ]);
    args
}

/// A Remotion composition rendered with `npx remotion render`.
fn remotion(spec: &Value, arguments: &Value, folder: &Path) -> Result<Vec<Value>, String> {
    let npx = crate::assistant::backends::search::find("npx")
        .ok_or("Remotion needs Node (npx isn't installed)")?;
    let project = spec["project"]
        .as_str()
        .map(|project| out::resolve(project, folder))
        .filter(|project| project.join("package.json").is_file())
        .ok_or("remotion needs project: a folder with a Remotion package.json")?;
    let composition = spec["composition"]
        .as_str()
        .ok_or("remotion needs composition: its id")?;
    if composition.starts_with('-') {
        return Err("remotion's composition is an id, not a flag".into());
    }
    // Rendering runs the project's own code and tools (`node_modules`,
    // its config) as the person, outside any sandbox: a job's or helper's
    // own project renders in its own shell instead, where its sandbox
    // holds.
    if out::serves_a_job() {
        let made_by_a_task = crate::assistant::agent::workspace::root()
            .into_iter()
            .chain([std::env::temp_dir()])
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| {
                project
                    .canonicalize()
                    .is_ok_and(|real| real.starts_with(&root))
            });
        if made_by_a_task {
            return Err(format!(
                "Render a Remotion project you made in your own shell: `npx remotion render {composition} out.mp4` in {}.",
                project.display()
            ));
        }
    }
    let target = out::output(
        arguments,
        "out",
        folder,
        &out::fresh_name(folder, composition, "mp4"),
    )?;
    let started = Instant::now();
    let mut command = crate::assistant::backends::search::command(&npx);
    command
        .current_dir(&project)
        .args(["--yes", "remotion", "render", composition])
        .arg(&target);
    if spec["props"].is_object() {
        command.arg(format!("--props={}", spec["props"]));
    }
    out::run(command, Duration::from_secs(900), "Remotion")?;
    let report = json!({"file": target, "took_s": out::took(started)});
    Ok(vec![json!({"type": "text", "text": report.to_string()})])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_sizes_and_recording_are_read_from_the_arguments() {
        let folder = std::env::temp_dir().join(format!("hex-make-args-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.png"), b"x").unwrap();
        assert_eq!(size(&json!({})).unwrap(), (1920, 1080));
        assert_eq!(size(&json!({"size": "1081x1921"})).unwrap(), (1080, 1920));
        assert!(size(&json!({"size": "big"})).is_err());
        let read = scenes(
            &json!({"scenes": [
                {"image": "a.png", "title": "Hallo", "zoom": true},
                {"color": "red", "duration": "0:03", "say": "Text"}
            ]}),
            &folder,
        )
        .unwrap();
        assert_eq!(read[0].source, Source::Image(folder.join("a.png")));
        assert_eq!(read[1].source, Source::Color("0xe62828".into()));
        assert_eq!(read[1].duration, 3.0);
        assert!(
            scenes(&json!({"scenes": [{"image": "none.png"}]}), &folder)
                .unwrap_err()
                .contains("no file")
        );
        assert!(scenes(&json!({}), &folder).is_err());
        assert_eq!(
            record_args(Path::new("/w/demo.mp4"), Some("https://example.com"), None),
            [
                "record",
                "start",
                "/w/demo.mp4",
                "https://example.com",
                "--fps",
                "30",
                "--cursor"
            ]
        );
        assert_eq!(
            record_args(Path::new("/w/d.mp4"), Some("javascript:x"), Some(90)).len(),
            6
        );
        assert!(
            call(&json!({"record": "start"}), &folder, None, &mut |_| {})
                .unwrap_err()
                .contains("task")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Makes a video from a picture, a color card, and a clip, with
    /// titles, captions, and music (all made here; files only).
    #[test]
    fn a_video_is_made_from_scenes_with_titles_and_music() {
        let Ok(ffmpeg) = tools::ffmpeg(&mut |_| {}) else {
            eprintln!("no ffmpeg: skipped");
            return;
        };
        let folder = std::env::temp_dir().join(format!("hex-make-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        image::RgbaImage::from_fn(800, 600, |x, y| {
            image::Rgba([(x / 4) as u8, (y / 3) as u8, 120, 255])
        })
        .save(folder.join("shot.png"))
        .unwrap();
        let mut command = out::ffmpeg_command(&ffmpeg);
        command
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=640x360:rate=25",
            ])
            .arg(folder.join("clip.mp4"));
        out::run(command, Duration::from_secs(60), "ffmpeg").unwrap();
        let mut command = out::ffmpeg_command(&ffmpeg);
        command
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=330:duration=3",
                "-af",
                "volume=0.2",
            ])
            .arg(folder.join("music.m4a"));
        out::run(command, Duration::from_secs(60), "ffmpeg").unwrap();
        let started = Instant::now();
        let content = call(
            &json!({"out": "demo.mp4", "size": "640x360", "fps": 25, "music": "music.m4a", "scenes": [
                {"color": "#1f2937", "title": "Agent-Desk Demo", "duration": 1.5},
                {"image": "shot.png", "text": "Ein Bild mit Untertitel", "duration": 2, "zoom": true},
                {"video": "clip.mp4", "text": "Ein Clip"}
            ]}),
            &folder,
            None,
            &mut |_| {},
        )
        .unwrap();
        let report: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
        println!("made in {:.1} s: {report}", started.elapsed().as_secs_f64());
        let length = report["duration_s"].as_f64().unwrap();
        assert!((length - 5.5).abs() < 0.3, "{report}");
        assert_eq!(report["scenes"], json!([1.5, 2.0, 2.0]));
        assert_eq!(
            content
                .iter()
                .filter(|item| item["type"] == "image")
                .count(),
            3
        );
        let meta = super::super::probe::probe(&ffmpeg, &folder.join("demo.mp4")).unwrap();
        assert_eq!((meta.width, meta.height), (640, 360));
        assert!(meta.audio.is_some(), "music became its sound");
        // The title card's frame shows the white title on the dark card.
        let first = image::open(report["frames"][0].as_str().unwrap())
            .unwrap()
            .to_rgb8();
        let bright = first
            .pixels()
            .filter(|pixel| pixel.0.iter().all(|c| *c > 200))
            .count();
        assert!(bright > 200, "{bright} bright pixels of the title");
        assert!(
            !folder
                .join(format!(".video-{}", std::process::id()))
                .exists(),
            "the work folder is gone"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
