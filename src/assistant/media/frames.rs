//! The pictures of a video: where its scenes change (ffmpeg's scene score
//! above [`SCENE_THRESHOLD`]), the strongest changes kept, gaps filled with
//! evenly spaced frames so every part is seen, each frame saved as a JPEG,
//! and one contact sheet of them all.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::assistant::backends::run;

/// How different a frame must be from the one before to start a scene.
pub const SCENE_THRESHOLD: f64 = 0.3;
/// The longest side of a saved frame.
const FRAME_EDGE: u32 = 960;
/// A contact sheet's columns and tile width.
const SHEET_COLUMNS: usize = 6;
const SHEET_TILE: u32 = 320;
/// Longer videos are scanned by their keyframes only (much faster).
const KEYFRAMES_ONLY_AFTER_S: f64 = 600.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Frame {
    /// Seconds into the video.
    pub at_s: f64,
    pub clock: String,
    /// Where a scene starts (else an evenly spaced frame).
    pub scene_change: bool,
    pub path: PathBuf,
}

/// A moment's `m:ss` (or `h:mm:ss`).
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, secs) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// The scene changes in `video` (seconds and score), within `window`.
pub fn scene_changes(
    ffmpeg: &Path,
    video: &Path,
    duration: f64,
    window: Option<(f64, f64)>,
    limit: Duration,
) -> Result<Vec<(f64, f64)>, String> {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-nostdin", "-nostats"]);
    if duration > KEYFRAMES_ONLY_AFTER_S {
        command.args(["-skip_frame", "nokey"]);
    }
    if let Some((start, end)) = window {
        command.args(["-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}")]);
    }
    command.arg("-i").arg(video).args([
        "-an",
        "-sn",
        "-vf",
        &format!("scale=160:-2,select='gt(scene,{SCENE_THRESHOLD})',metadata=print"),
        "-fps_mode",
        "vfr",
        "-f",
        "null",
        "-",
    ]);
    let ran = run::bounded(command, limit).map_err(|error| error.to_string())?;
    let offset = window.map_or(0.0, |(start, _)| start);
    let found: Vec<(f64, f64)> = parse_scenes(&ran.stderr)
        .into_iter()
        .map(|(at, score)| (at + offset, score))
        .collect();
    if !ran.success() && found.is_empty() && !ran.timed_out() {
        return Err(format!(
            "ffmpeg couldn't scan the video: {}",
            ran.last_words()
        ));
    }
    Ok(found)
}

/// `metadata=print`'s lines as (seconds, scene score).
pub fn parse_scenes(text: &str) -> Vec<(f64, f64)> {
    let mut found = Vec::new();
    let mut at: Option<f64> = None;
    for line in text.lines() {
        if let Some(time) = line.split("pts_time:").nth(1) {
            at = time
                .split_whitespace()
                .next()
                .and_then(|time| time.parse().ok());
        } else if let Some(score) = line.split("lavfi.scene_score=").nth(1)
            && let (Some(time), Ok(score)) = (at.take(), score.trim().parse::<f64>())
        {
            found.push((time, score));
        }
    }
    found
}

/// The moments to show, in order: the strongest scene changes (at most
/// `max` less the fill), and evenly spaced frames wherever no scene change
/// is near, so every part of the video (or of `window`) is seen.
pub fn choose(
    scenes: &[(f64, f64)],
    duration: f64,
    window: Option<(f64, f64)>,
    max: usize,
) -> Vec<(f64, bool)> {
    let (start, end) = window.unwrap_or((0.0, duration));
    let span = (end - start).max(0.1);
    let max = max.max(2);
    // Even frames: about one per 10 s, between 6 and half the frames.
    let most = (max / 2).max(1);
    let even = ((span / 10.0).ceil() as usize).clamp(6.min(most), most);
    let mut strongest: Vec<(f64, f64)> = scenes
        .iter()
        .copied()
        .filter(|(at, _)| *at >= start && *at <= end)
        .collect();
    strongest.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut chosen: Vec<(f64, bool)> = Vec::new();
    // Two scene changes closer than this show the same thing.
    let near = (span / (max as f64 * 2.0)).max(0.4);
    for (at, _) in strongest {
        if chosen.len() >= max - even.min(max - 1) {
            break;
        }
        if chosen.iter().all(|(other, _)| (other - at).abs() >= near) {
            chosen.push((at, true));
        }
    }
    let gap = span / even as f64;
    for index in 0..even {
        if chosen.len() >= max {
            break;
        }
        let at = start + gap * (index as f64 + 0.5);
        if chosen
            .iter()
            .all(|(other, _)| (other - at).abs() >= gap / 2.0)
        {
            chosen.push((at, false));
        }
    }
    chosen.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    chosen
}

/// Saves the frame at each moment as `frame-NN.jpg` in `dir`, four at a
/// time, until `deadline`.
pub fn extract(
    ffmpeg: &Path,
    video: &Path,
    moments: &[(f64, bool)],
    dir: &Path,
    deadline: Instant,
) -> Vec<Frame> {
    let jobs: Vec<(usize, f64, bool)> = moments
        .iter()
        .enumerate()
        .map(|(index, (at, scene))| (index + 1, *at, *scene))
        .collect();
    let mut frames: Vec<Frame> = std::thread::scope(|scope| {
        jobs.chunks(jobs.len().div_ceil(4).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|(number, at, scene)| {
                            let left = deadline.saturating_duration_since(Instant::now());
                            if left.is_zero() {
                                return None;
                            }
                            let path = dir.join(format!("frame-{number:02}.jpg"));
                            one(ffmpeg, video, *at, &path, left.min(Duration::from_secs(15))).then(
                                || Frame {
                                    at_s: (*at * 10.0).round() / 10.0,
                                    clock: clock(*at),
                                    scene_change: *scene,
                                    path,
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    frames.sort_by(|a, b| {
        a.at_s
            .partial_cmp(&b.at_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    frames
}

fn one(ffmpeg: &Path, video: &Path, at: f64, path: &Path, limit: Duration) -> bool {
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-ss", &format!("{at:.3}"), "-i"])
        .arg(video)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!(
                "scale='if(gt(iw,ih),min({FRAME_EDGE},iw),-2)':'if(gt(iw,ih),-2,min({FRAME_EDGE},ih))'"
            ),
            "-q:v",
            "4",
        ])
        .arg(path);
    run::bounded(command, limit).is_ok_and(|ran| ran.success()) && path.is_file()
}

/// One picture of every frame, [`SHEET_COLUMNS`] to a row, in order.
pub fn contact_sheet(ffmpeg: &Path, frames: &[Frame], dir: &Path) -> Option<PathBuf> {
    if frames.len() < 2 {
        return None;
    }
    // The tile filter reads a numbered sequence: link the frames as one.
    let sequence = dir.join("sheet-input");
    let _ = std::fs::remove_dir_all(&sequence);
    std::fs::create_dir_all(&sequence).ok()?;
    for (index, frame) in frames.iter().enumerate() {
        std::fs::copy(&frame.path, sequence.join(format!("{:02}.jpg", index + 1))).ok()?;
    }
    let rows = frames.len().div_ceil(SHEET_COLUMNS);
    let columns = frames.len().min(SHEET_COLUMNS);
    let sheet = dir.join("contact-sheet.jpg");
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"])
        .args(["-start_number", "1", "-i"])
        .arg(sequence.join("%02d.jpg"))
        .args([
            "-vf",
            &format!(
                "scale={SHEET_TILE}:{SHEET_TILE}:force_original_aspect_ratio=decrease,pad={SHEET_TILE}:{SHEET_TILE}:(ow-iw)/2:(oh-ih)/2,tile={columns}x{rows}:padding=4:margin=4"
            ),
            "-frames:v",
            "1",
            "-q:v",
            "4",
        ])
        .arg(&sheet);
    let made = run::bounded(command, Duration::from_secs(20)).is_ok_and(|ran| ran.success());
    let _ = std::fs::remove_dir_all(&sequence);
    (made && sheet.is_file()).then_some(sheet)
}

/// A time range in the person's words for where to look (`1:20-1:45`,
/// `ab 2:10`, `0:30`): seconds, a single moment widened to ±10 s.
pub fn window_in(focus: &str, duration: f64) -> Option<(f64, f64)> {
    let times: Vec<f64> = focus
        .split(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .filter(|token| token.contains(':'))
        .filter_map(|token| super::probe::clock_seconds(token.trim_matches(['.', ':'])))
        .collect();
    let (start, end) = match times.as_slice() {
        [] => return None,
        [at] => (at - 10.0, at + 10.0),
        [start, end, ..] if end > start => (*start, *end),
        [start, ..] => (start - 10.0, start + 10.0),
    };
    let (start, end) = (start.max(0.0), end.min(duration.max(0.1)));
    (end - start >= 1.0).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_scores_are_read_from_ffmpeg_s_log() {
        let log = "[Parsed_metadata_2 @ 0x1] frame:0    pts:4096    pts_time:2.13333\n\
                   [Parsed_metadata_2 @ 0x1] lavfi.scene_score=0.512\n\
                   [Parsed_metadata_2 @ 0x1] frame:1    pts:9000    pts_time:4.5\n\
                   [Parsed_metadata_2 @ 0x1] lavfi.scene_score=0.91\n";
        assert_eq!(parse_scenes(log), [(2.13333, 0.512), (4.5, 0.91)]);
        assert!(parse_scenes("frame= 10 fps=0.0").is_empty());
    }

    #[test]
    fn the_strongest_changes_are_kept_and_the_gaps_filled() {
        // Forty cuts in three minutes: at most 24 pictures, the strongest
        // cuts first, and evenly spaced ones where no cut is near.
        let scenes: Vec<(f64, f64)> = (0..40)
            .map(|index| (index as f64 * 4.5 + 1.0, 0.3 + index as f64 / 100.0))
            .collect();
        let chosen = choose(&scenes, 180.0, None, 24);
        assert!(chosen.len() <= 24 && chosen.len() >= 18, "{}", chosen.len());
        assert!(
            chosen.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "in order"
        );
        assert!(chosen.iter().any(|(_, scene)| *scene));
        assert!(
            chosen.iter().any(|(at, _)| *at > 150.0),
            "the end is seen too"
        );
        // No cuts at all (one long shot): evenly spaced frames.
        let still = choose(&[], 60.0, None, 24);
        assert_eq!(still.len(), 6);
        assert!(still.iter().all(|(_, scene)| !scene));
        assert!((still[0].0 - 5.0).abs() < 1e-9);
        // A window only looks there.
        let focused = choose(&scenes, 180.0, Some((60.0, 90.0)), 12);
        assert!(focused.iter().all(|(at, _)| (60.0..=90.0).contains(at)));
    }

    #[test]
    fn moments_read_as_clocks_and_focus_as_a_window() {
        assert_eq!(clock(0.4), "0:00");
        assert_eq!(clock(83.0), "1:23");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(
            window_in("was passiert bei 1:20-1:45?", 180.0),
            Some((80.0, 105.0))
        );
        assert_eq!(window_in("ab 0:05", 180.0), Some((0.0, 15.0)));
        assert_eq!(window_in("is it AI-generated?", 180.0), None);
        assert_eq!(window_in("2:50 to 3:30", 180.0), Some((170.0, 180.0)));
    }
}
