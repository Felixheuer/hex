//! A video file's facts from ffmpeg's own description of its input
//! (`ffmpeg -i`): duration, resolution, frame rate, codecs, the audio, and
//! the container's tags (an encoder or the software that made it can say
//! how a video was produced). No ffprobe needed.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

use crate::assistant::backends::run;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Meta {
    pub duration_s: f64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub video_codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<Audio>,
    /// The container's tags (`encoder`, `creation_time`, `comment`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Audio {
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<String>,
}

/// `video`'s facts.
pub fn probe(ffmpeg: &Path, video: &Path) -> Result<Meta, String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-nostdin", "-i"]).arg(video);
    let ran = run::bounded(command, Duration::from_secs(20)).map_err(|error| error.to_string())?;
    let meta = parse(&ran.stderr);
    if meta.duration_s <= 0.0 && meta.width == 0 {
        return Err(format!(
            "ffmpeg can't read {}: {}",
            video.display(),
            ran.last_words()
        ));
    }
    Ok(meta)
}

/// `ffmpeg -i`'s description of its first input.
pub fn parse(text: &str) -> Meta {
    let mut meta = Meta::default();
    let mut in_tags = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Output #") {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Duration:") {
            in_tags = false;
            meta.duration_s = rest
                .split(',')
                .next()
                .and_then(|clock| clock_seconds(clock.trim()))
                .unwrap_or(0.0);
            meta.bitrate_kbps = rest
                .split("bitrate:")
                .nth(1)
                .and_then(|rate| rate.trim().split(' ').next()?.parse().ok());
            continue;
        }
        if trimmed == "Metadata:" {
            // Only the container's own tags: they come before Duration.
            in_tags = meta.duration_s == 0.0 && meta.width == 0;
            continue;
        }
        if trimmed.starts_with("Stream #") {
            in_tags = false;
            if let Some(video) = trimmed.split("Video: ").nth(1) {
                if meta.width == 0 {
                    meta.video_codec = word(video);
                    if let Some((width, height)) = resolution(video) {
                        meta.width = width;
                        meta.height = height;
                    }
                    meta.fps = video
                        .split(", ")
                        .find_map(|part| part.strip_suffix(" fps")?.trim().parse().ok());
                }
            } else if let Some(audio) = trimmed.split("Audio: ").nth(1)
                && meta.audio.is_none()
            {
                let parts: Vec<&str> = audio.split(", ").collect();
                meta.audio = Some(Audio {
                    codec: word(audio).unwrap_or_default(),
                    sample_rate: parts
                        .iter()
                        .find_map(|part| part.strip_suffix(" Hz")?.trim().parse().ok()),
                    channels: parts.get(2).map(|channels| (*channels).to_owned()),
                });
            }
            continue;
        }
        if in_tags
            && let Some((key, value)) = trimmed.split_once(':')
            && meta.tags.len() < 16
        {
            let (key, value) = (key.trim(), value.trim());
            if !key.is_empty() && !value.is_empty() && !key.contains(' ') {
                meta.tags
                    .push((key.to_owned(), value.chars().take(200).collect()));
            }
        }
    }
    meta
}

/// `00:03:01.23` as seconds.
pub fn clock_seconds(clock: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in clock.split(':') {
        seconds = seconds * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    Some(seconds)
}

fn word(text: &str) -> Option<String> {
    text.split([' ', ','])
        .next()
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
}

fn resolution(text: &str) -> Option<(u32, u32)> {
    text.split([' ', ','])
        .filter_map(|token| {
            let (width, height) = token.split_once('x')?;
            Some((width.parse().ok()?, height.parse().ok()?))
        })
        .find(|(width, height): &(u32, u32)| *width >= 16 && *height >= 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIBED: &str = r"Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'video.mp4':
  Metadata:
    major_brand     : isom
    minor_version   : 512
    encoder         : Lavf61.7.100
    comment         : Made with an AI video tool
  Duration: 00:03:01.23, start: 0.000000, bitrate: 1543 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1280x720 [SAR 1:1 DAR 16:9], 1410 kb/s, 29.97 fps, 29.97 tbr, 30k tbn (default)
      Metadata:
        handler_name    : VideoHandler
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 128 kb/s (default)
      Metadata:
        handler_name    : SoundHandler
At least one output file must be specified";

    #[test]
    fn ffmpeg_s_description_reads_as_facts() {
        let meta = parse(DESCRIBED);
        assert!((meta.duration_s - 181.23).abs() < 1e-6);
        assert_eq!((meta.width, meta.height), (1280, 720));
        assert_eq!(meta.fps, Some(29.97));
        assert_eq!(meta.video_codec.as_deref(), Some("h264"));
        assert_eq!(meta.bitrate_kbps, Some(1543));
        let audio = meta.audio.unwrap();
        assert_eq!(audio.codec, "aac");
        assert_eq!(audio.sample_rate, Some(44100));
        assert_eq!(audio.channels.as_deref(), Some("stereo"));
        assert!(
            meta.tags
                .contains(&("encoder".into(), "Lavf61.7.100".into()))
        );
        assert!(meta.tags.iter().any(|(key, _)| key == "comment"));
        assert!(
            !meta.tags.iter().any(|(key, _)| key == "handler_name"),
            "only the container's tags"
        );
        assert_eq!(clock_seconds("01:00:00.5"), Some(3600.5));
        assert_eq!(clock_seconds("N/A"), None);
        // A file without audio, or not a video at all.
        let silent = parse(
            "  Duration: 00:00:05.00, start: 0.0, bitrate: 90 kb/s\n  Stream #0:0: Video: vp9, yuv420p, 640x360, 25 fps, 25 tbr",
        );
        assert!(silent.audio.is_none() && silent.width == 640);
        assert_eq!(parse("nothing").width, 0);
    }
}
