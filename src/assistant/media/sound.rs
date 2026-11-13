//! How a video sounds: its audio as 16 kHz mono (one ffmpeg pass that also
//! measures loudness, EBU R128), what that is like (level, silence, a steady
//! beat and its tempo, so music and speech are told apart), and what is said,
//! transcribed by Hex's own engine: the running Hex's transcription service
//! (`POST /transcriptions`, the same model the person dictates with), cut at
//! quiet moments into clips of at most 30 s so each runs on the loaded
//! dictation engine and carries its time. Without a running Hex, Linux and
//! Windows load the model in this process as `hex transcribe` does.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::captions::Line;
use crate::assistant::backends::run;

pub const RATE: u32 = 16_000;
/// The most audio heard (and transcribed).
pub const MAX_SECONDS: f64 = 1200.0;
/// The longest clip sent at once: up to this, the dictation engine's loaded
/// copy transcribes it.
const CLIP_SECONDS: f64 = 30.0;

/// The audio, and its loudness as ffmpeg measured it.
pub struct Heard {
    pub samples: Vec<f32>,
    pub lufs: Option<f64>,
    pub loudness_range: Option<f64>,
}

/// `video`'s first audio track as 16 kHz mono samples.
pub fn extract(ffmpeg: &Path, video: &Path, dir: &Path, limit: Duration) -> Result<Heard, String> {
    let raw = dir.join("audio.pcm");
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-nostats", "-y", "-i"])
        .arg(video)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-t",
            &format!("{MAX_SECONDS}"),
            "-af",
            "ebur128=framelog=quiet",
            "-ac",
            "1",
            "-ar",
            &RATE.to_string(),
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
        ])
        .arg(&raw);
    let ran = run::bounded(command, limit).map_err(|error| error.to_string())?;
    let bytes = std::fs::read(&raw).unwrap_or_default();
    let _ = std::fs::remove_file(&raw);
    if !ran.success() && bytes.is_empty() {
        return Err(format!(
            "ffmpeg couldn't read the sound: {}",
            ran.last_words()
        ));
    }
    let samples = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect();
    let (lufs, loudness_range) = loudness(&ran.stderr);
    Ok(Heard {
        samples,
        lufs,
        loudness_range,
    })
}

/// The integrated loudness (LUFS) and loudness range (LU) of ebur128's
/// summary.
pub fn loudness(text: &str) -> (Option<f64>, Option<f64>) {
    let Some((_, summary)) = text.rsplit_once("Summary:") else {
        return (None, None);
    };
    let value = |key: &str| {
        summary.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(key)?;
            rest.split_whitespace().next()?.parse::<f64>().ok()
        })
    };
    (value("I:"), value("LRA:"))
}

/// What the sound is like.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Character {
    /// `speech`, `music`, `speech over music`, `quiet`, or `sound` (neither).
    pub kind: String,
    pub level_dbfs: f64,
    pub peak_dbfs: f64,
    /// The share of 50 ms stretches that are near silence.
    pub silent_share: f64,
    /// A steady beat's tempo, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tempo_bpm: Option<f64>,
    /// How regular the beat is, 0 to 1.
    pub beat_strength: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness_lufs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness_range_lu: Option<f64>,
    /// Words heard per minute of sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words_per_minute: Option<f64>,
}

fn db(value: f64) -> f64 {
    if value <= 1e-9 {
        -120.0
    } else {
        (20.0 * value.log10() * 10.0).round() / 10.0
    }
}

/// Level, silence, and beat of `samples` (16 kHz mono); `words` heard in
/// them decide speech.
pub fn character(samples: &[f32], words: Option<usize>) -> Character {
    if samples.is_empty() {
        return Character {
            kind: "quiet".into(),
            level_dbfs: -120.0,
            peak_dbfs: -120.0,
            silent_share: 1.0,
            ..Character::default()
        };
    }
    let energy: f64 = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    let window = (RATE / 20) as usize;
    let stretches: Vec<f64> = samples
        .chunks(window)
        .map(|chunk| {
            (chunk.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / chunk.len() as f64).sqrt()
        })
        .collect();
    let silent = stretches.iter().filter(|rms| db(**rms) < -45.0).count();
    let silent_share = silent as f64 / stretches.len() as f64;
    let (tempo, strength) = beat(samples);
    let minutes = samples.len() as f64 / f64::from(RATE) / 60.0;
    let words_per_minute = words.map(|words| (words as f64 / minutes.max(1.0 / 60.0)).round());
    let speech = words_per_minute.is_some_and(|rate| rate >= 40.0);
    let music = strength >= 0.35 && silent_share < 0.3;
    let kind = match (speech, music) {
        _ if silent_share > 0.9 => "quiet",
        (true, true) => "speech over music",
        (true, false) => "speech",
        (false, true) => "music",
        (false, false) => "sound",
    };
    Character {
        kind: kind.into(),
        level_dbfs: db((energy / samples.len() as f64).sqrt()),
        peak_dbfs: db(f64::from(peak)),
        silent_share: (silent_share * 100.0).round() / 100.0,
        tempo_bpm: (strength >= 0.2).then_some(tempo).flatten(),
        beat_strength: (strength * 100.0).round() / 100.0,
        words_per_minute,
        ..Character::default()
    }
}

/// The tempo of the steadiest beat between 60 and 180 BPM, and how regular
/// it is: the onset envelope (rises in 10 ms energy) correlated with
/// itself.
pub fn beat(samples: &[f32]) -> (Option<f64>, f64) {
    let hop = (RATE / 100) as usize;
    let energy: Vec<f64> = samples
        .chunks(hop)
        .map(|chunk| (chunk.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() + 1e-10).ln())
        .collect();
    // Rises of about 1 dB or more in 10 ms are onsets; a steady tone's
    // ripple isn't.
    let mut onsets: Vec<f64> = energy
        .windows(2)
        .map(|pair| (pair[1] - pair[0] - 0.2).max(0.0))
        .collect();
    if onsets.len() < 400 {
        return (None, 0.0);
    }
    let mean = onsets.iter().sum::<f64>() / onsets.len() as f64;
    for onset in &mut onsets {
        *onset -= mean;
    }
    let zero: f64 = onsets.iter().map(|value| value * value).sum();
    if zero <= 1e-9 {
        return (None, 0.0);
    }
    let correlation = |lag: usize| -> f64 {
        onsets
            .iter()
            .zip(&onsets[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / zero
    };
    // 100 envelope values a second: 60 BPM is a lag of 100, 180 of 33.
    let (lag, best) =
        (33..=100)
            .map(|lag| (lag, correlation(lag)))
            .fold(
                (0, f64::MIN),
                |best, next| if next.1 > best.1 { next } else { best },
            );
    if lag == 0 {
        return (None, 0.0);
    }
    let bpm = (6000.0 / lag as f64 * 10.0).round() / 10.0;
    (Some(bpm), best.clamp(0.0, 1.0))
}

/// Where to cut `samples` into clips of at most [`CLIP_SECONDS`]: the
/// quietest 50 ms in each clip's last five seconds.
pub fn cuts(samples: &[f32]) -> Vec<(usize, usize)> {
    let clip = (CLIP_SECONDS * f64::from(RATE)) as usize;
    let look = 5 * RATE as usize;
    let window = (RATE / 20) as usize;
    let mut cuts = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        if samples.len() - start <= clip {
            cuts.push((start, samples.len()));
            break;
        }
        let end = start + clip;
        let quietest = (end - look..end - window)
            .step_by(window)
            .min_by(|a, b| {
                let level = |at: &usize| -> f64 {
                    samples[*at..*at + window]
                        .iter()
                        .map(|s| f64::from(*s).powi(2))
                        .sum()
                };
                level(a)
                    .partial_cmp(&level(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(end, |at| at + window / 2);
        cuts.push((start, quietest));
        start = quietest;
    }
    cuts
}

/// 16-bit mono WAV of `samples`.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// What is said in `samples`, clip by clip with its time, until `deadline`
/// (the rest is left out and the second value says so).
pub fn transcribe(samples: &[f32], deadline: Instant) -> Result<(Vec<Line>, bool), String> {
    let engine = Engine::here()?;
    let mut lines = Vec::new();
    for (start, end) in cuts(samples) {
        if Instant::now() >= deadline {
            return Ok((lines, true));
        }
        let clip = &samples[start..end];
        // A clip that is all but silent says nothing.
        if clip.iter().all(|sample| sample.abs() < 0.003) {
            continue;
        }
        let text = engine.transcribe(&wav(clip))?;
        let text = text.trim();
        if !text.is_empty() {
            lines.push(Line {
                at_s: (start as f64 / f64::from(RATE) * 10.0).round() / 10.0,
                text: text.to_owned(),
            });
        }
    }
    Ok((lines, false))
}

/// Hex's transcription engine, as this process reaches it.
struct Engine {
    selection: crate::transcription::models::TranscriptionSelection,
    running: bool,
}

impl Engine {
    fn here() -> Result<Self, String> {
        let settings = crate::app_settings::AppSettings::read()
            .map_err(|error| format!("couldn't read Hex's settings: {error:#}"))?;
        let selection =
            crate::control::model_selection(&settings, settings.transcription.model.as_str(), None)
                .map_err(|error| format!("Hex's transcription model isn't usable: {error:?}"))?;
        let running = crate::local_api::running_pid().is_some();
        if !running && cfg!(target_os = "macos") {
            return Err("Hex isn't running, so its transcription engine can't listen".into());
        }
        Ok(Self { selection, running })
    }

    fn transcribe(&self, wav: &[u8]) -> Result<String, String> {
        if !self.running {
            return offline(wav, &self.selection);
        }
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("model", self.selection.model.as_str())
            .append_pair("language", &self.selection.language)
            .finish();
        match crate::local_api::post_wav(
            &format!("/transcriptions?{query}"),
            wav,
            Duration::from_secs(120),
            1024 * 1024,
        ) {
            Ok((200, body)) => serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|reply| reply["transcript"].as_str().map(str::to_owned))
                .ok_or_else(|| "Hex sent an unreadable transcript".into()),
            Ok((status, body)) => Err(format!(
                "Hex's transcription answered {status} ({})",
                crate::local_api::error_code(status, &body)
            )),
            Err(error) => Err(format!("couldn't reach Hex's transcription: {error:#}")),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn offline(
    wav: &[u8],
    selection: &crate::transcription::models::TranscriptionSelection,
) -> Result<String, String> {
    crate::transcription::local_transcriber::transcribe_wav_bytes(wav, selection)
        .map_err(|error| format!("couldn't transcribe it: {error:#}"))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn offline(
    _: &[u8],
    _: &crate::transcription::models::TranscriptionSelection,
) -> Result<String, String> {
    Err("Hex isn't running, so its transcription engine can't listen".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(count: f64) -> usize {
        (count * f64::from(RATE)) as usize
    }

    #[test]
    fn a_steady_beat_is_music_and_a_tone_or_silence_is_not() {
        // Clicks at 120 BPM over a quiet hum.
        let mut clicks = vec![0.0_f32; seconds(20.0)];
        for (index, sample) in clicks.iter_mut().enumerate() {
            *sample = 0.01 * (index as f32 * 0.05).sin();
        }
        for beat in 0..40 {
            let at = seconds(beat as f64 * 0.5);
            for offset in 0..400 {
                if let Some(sample) = clicks.get_mut(at + offset) {
                    *sample += 0.6 * (1.0 - offset as f32 / 400.0) * ((offset as f32) * 0.7).sin();
                }
            }
        }
        let (tempo, strength) = beat(&clicks);
        let tempo = tempo.unwrap();
        assert!(
            (tempo - 120.0).abs() < 3.0 || (tempo - 60.0).abs() < 2.0,
            "{tempo}"
        );
        assert!(strength > 0.35, "{strength}");
        let heard = character(&clicks, Some(0));
        assert_eq!(heard.kind, "music");
        assert_eq!(character(&clicks, Some(60)).kind, "speech over music");

        let tone: Vec<f32> = (0..seconds(10.0))
            .map(|index| 0.5 * (index as f32 * 2.0 * std::f32::consts::PI * 440.0 / 16_000.0).sin())
            .collect();
        let heard = character(&tone, Some(0));
        assert_eq!(heard.kind, "sound");
        assert!(heard.tempo_bpm.is_none(), "{heard:?}");
        assert!(
            (heard.level_dbfs - -9.0).abs() < 0.5,
            "{}",
            heard.level_dbfs
        );
        assert_eq!(character(&vec![0.0; seconds(5.0)], None).kind, "quiet");
        assert_eq!(character(&[], None).kind, "quiet");
        assert_eq!(character(&tone, Some(30)).kind, "speech");
    }

    #[test]
    fn clips_end_at_quiet_moments_and_become_wavs() {
        let mut samples = vec![0.3_f32; seconds(70.0)];
        // A pause at 27 s.
        for sample in &mut samples[seconds(27.0)..seconds(27.2)] {
            *sample = 0.0;
        }
        let cuts = cuts(&samples);
        assert_eq!(cuts.len(), 3, "{cuts:?}");
        assert_eq!(cuts[0].0, 0);
        let first_end = cuts[0].1 as f64 / f64::from(RATE);
        assert!((27.0..27.2).contains(&first_end), "{first_end}");
        assert!(cuts.windows(2).all(|pair| pair[0].1 == pair[1].0));
        assert_eq!(cuts.last().unwrap().1, samples.len());
        assert!(cuts.iter().all(|(start, end)| end - start <= seconds(30.0)));
        let bytes = wav(&samples[..160]);
        assert_eq!(bytes.len(), 44 + 320);
        assert!(
            crate::transcription::service::AudioClip::decode_wav(std::io::Cursor::new(bytes))
                .is_ok()
        );
    }

    #[test]
    fn ebur128_s_summary_reads_as_loudness() {
        let log = "[Parsed_ebur128_0 @ 0x1] t: 1.0 M: -20 S: -21 I: -19.0 LUFS\n\
            [Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -16.4 LUFS\n    Threshold: -26.8 LUFS\n\n  Loudness range:\n    LRA:         6.2 LU\n";
        assert_eq!(loudness(log), (Some(-16.4), Some(6.2)));
        assert_eq!(loudness("no summary"), (None, None));
    }
}
