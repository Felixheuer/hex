//! `hex_audio`: sound files without a sound device. Transcribe one with
//! Agent-Desk's own engine (the running app's `/transcriptions`, as
//! `hex_watch_video` does), inspect it, cut, convert, normalize, mix, or
//! join files with ffmpeg, and speak text into a file with Agent-Desk's own
//! voice (Supertonic; the system voice through `say -o` when it isn't
//! installed). Nothing here plays or records sound.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::captions::Line;
use super::{out, probe, sound, tools};

pub const NAME: &str = "hex_audio";

/// How long one call may take: transcription runs until then and says
/// where it stopped.
const TRANSCRIBE_BUDGET: Duration = Duration::from_secs(240);
const FFMPEG_LIMIT: Duration = Duration::from_secs(300);
const SPEAK_LIMIT: Duration = Duration::from_secs(240);
/// The most text spoken into one file.
const MAX_SPEAK_CHARS: usize = 20_000;

pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "Work with sound files (MP3, M4A, WAV, AIFF, OGG, FLAC, or a video's sound): op transcribe (what is said, with times, by Agent-Desk's own speech recognition; start/end pick a part), inspect (duration, format, loudness, speech or music), trim (start, end), convert (to out's format), normalize (even loudness, target_lufs default -16), mix (tracks played together, each with volume and delay), join (tracks one after another), speak (text read aloud into a file with Agent-Desk's voice; voice F1-F5 or M1-M5). Results are written into your folder; nothing is played.",
        "inputSchema": {"type": "object", "properties": {
            "op": {"type": "string", "enum": ["transcribe", "inspect", "trim", "convert", "normalize", "mix", "join", "speak"]},
            "path": {"type": "string", "description": "The sound (or video) file."},
            "out": {"type": "string", "description": "Where to write the result; its extension picks the format (default .m4a)."},
            "start": {"type": ["string", "number"], "description": "A time like 1:20 or 80 (seconds)."},
            "end": {"type": ["string", "number"]},
            "target_lufs": {"type": "number"},
            "tracks": {"type": "array", "description": "mix/join: [{\"path\", \"volume\" (1 = as is), \"delay\" (seconds, mix)}] or paths.", "items": {}},
            "text": {"type": "string", "description": "speak: what to say."},
            "voice": {"type": "string", "description": "speak: F1-F5 (women) or M1-M5 (men); default M1."},
            "language": {"type": "string", "description": "speak/transcribe: the language code, e.g. de or en (default: detected)."}
        }, "required": ["op"]},
    })
}

/// `path`'s sound from `start` for `length` seconds as 16 kHz mono.
fn samples(
    ffmpeg: &Path,
    path: &Path,
    start: f64,
    length: f64,
    limit: Duration,
) -> Result<Vec<f32>, String> {
    let mut command = out::ffmpeg_command(ffmpeg);
    command
        .args(["-ss", &format!("{start:.3}"), "-i"])
        .arg(path)
        .args([
            "-t",
            &format!("{:.3}", length.min(sound::MAX_SECONDS)),
            "-vn",
            "-ac",
            "1",
            "-ar",
            &sound::RATE.to_string(),
            "-f",
            "s16le",
            "-",
        ]);
    command.stderr(std::process::Stdio::null());
    let output = run_capture(command, limit)?;
    Ok(output
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect())
}

/// Runs `command` for at most `limit` and returns its stdout bytes.
fn run_capture(mut command: Command, limit: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    command.stdout(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("ffmpeg: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("ffmpeg gave no output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let bytes = reader.join().unwrap_or_default();
            if !status.success() && bytes.is_empty() {
                return Err("ffmpeg couldn't read the sound".into());
            }
            return Ok(bytes);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("ffmpeg took more than {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The tracks `mix` and `join` take: a path and its volume and delay.
fn tracks(arguments: &Value, folder: &Path) -> Result<Vec<(PathBuf, f64, f64)>, String> {
    let list = arguments["tracks"]
        .as_array()
        .filter(|list| list.len() >= 2)
        .ok_or("mix and join need tracks: two or more files")?;
    list.iter()
        .map(|track| {
            let (path, volume, delay) = match track {
                Value::String(path) => (path.as_str(), 1.0, 0.0),
                track => (
                    track["path"].as_str().ok_or("each track needs a path")?,
                    track["volume"].as_f64().unwrap_or(1.0).clamp(0.0, 4.0),
                    out::seconds(&track["delay"]).unwrap_or(0.0),
                ),
            };
            let resolved = out::resolve(path, folder);
            if resolved.is_file() {
                Ok((resolved, volume, delay))
            } else {
                Err(format!("There is no file “{path}”."))
            }
        })
        .collect()
}

/// The language to speak `text` in: the one given, else the text's own,
/// else the system's; as Supertonic's code when it reads it.
fn language(arguments: &Value, text: &str) -> String {
    arguments["language"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| crate::assistant::speech::language_of(text).map(str::to_owned))
        .unwrap_or_else(crate::assistant::speech::system_language)
}

/// `text` spoken into `target`: Agent-Desk's voice when installed and it
/// reads the language, else `say` (macOS). Which voice spoke.
pub fn speak(
    text: &str,
    voice: &str,
    language_code: &str,
    target: &Path,
    ffmpeg: &Path,
) -> Result<String, String> {
    use crate::assistant::speech::neural;
    let text: String = text.chars().take(MAX_SPEAK_CHARS).collect();
    let preset = neural::PRESETS
        .iter()
        .copied()
        .find(|preset| preset.eq_ignore_ascii_case(voice.trim_start_matches(neural::PREFIX)))
        .unwrap_or(neural::DEFAULT_PRESET);
    let wav = target.with_extension("voice.wav");
    let spoke = match neural::text::language_code(language_code) {
        Some(code) if neural::files::installed() => {
            let (rate, samples) = neural::render(&text, code, preset)
                .map_err(|error| format!("Agent-Desk's voice couldn't speak: {error:#}"))?;
            write_wav(&wav, rate, &samples)?;
            format!("Agent-Desk voice {preset}")
        }
        _ if cfg!(target_os = "macos") => {
            let aiff = target.with_extension("voice.aiff");
            let mut command = Command::new("/usr/bin/say");
            command.arg("-o").arg(&aiff).arg("--").arg(&text);
            out::run(command, SPEAK_LIMIT, "say")?;
            let mut convert = out::ffmpeg_command(ffmpeg);
            convert.arg("-i").arg(&aiff).arg(&wav);
            let converted = out::run(convert, FFMPEG_LIMIT, "ffmpeg");
            let _ = std::fs::remove_file(&aiff);
            converted?;
            "the system voice (Agent-Desk's voice isn't installed or doesn't read this language)"
                .into()
        }
        _ => {
            return Err("Agent-Desk's voice isn't installed (Settings › Assistant › Voice)".into());
        }
    };
    if out::extension(target) == "wav" {
        std::fs::rename(&wav, target).map_err(|error| error.to_string())?;
    } else {
        let mut command = out::ffmpeg_command(ffmpeg);
        command.arg("-i").arg(&wav).arg(target);
        let converted = out::run(command, FFMPEG_LIMIT, "ffmpeg");
        let _ = std::fs::remove_file(&wav);
        converted?;
    }
    Ok(spoke)
}

/// 16-bit mono WAV of `samples` at `rate`.
pub fn write_wav(path: &Path, rate: u32, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|error| error.to_string())?;
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)
            .map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())
}

/// A file's duration in seconds, by ffmpeg.
pub fn duration(ffmpeg: &Path, path: &Path) -> Option<f64> {
    probe::probe(ffmpeg, path)
        .ok()
        .map(|meta| meta.duration_s)
        .filter(|seconds| *seconds > 0.0)
}

pub fn call(
    arguments: &Value,
    folder: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    let started = Instant::now();
    let op = arguments["op"].as_str().unwrap_or_default();
    let ffmpeg = tools::ffmpeg(progress)?;
    let mut report = json!({"op": op});
    let stem = |path: &Path| {
        path.file_stem().map_or_else(
            || "audio".into(),
            |stem| stem.to_string_lossy().into_owned(),
        )
    };
    let target = |default_stem: &str, default_extension: &str| {
        out::output(
            arguments,
            "out",
            folder,
            &out::fresh_name(folder, default_stem, default_extension),
        )
    };
    let start = out::seconds(&arguments["start"]).unwrap_or(0.0);
    let end = out::seconds(&arguments["end"]);
    match op {
        "speak" => {
            let text = arguments["text"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or("speak needs text")?;
            let file = target("speech", "m4a")?;
            progress("Speaking the text into a file");
            let voice = speak(
                text,
                arguments["voice"].as_str().unwrap_or("M1"),
                &language(arguments, text),
                &file,
                &ffmpeg,
            )?;
            report["voice"] = json!(voice);
            report["out"] = json!(file);
            report["duration_s"] =
                json!(duration(&ffmpeg, &file).map(|seconds| (seconds * 10.0).round() / 10.0));
        }
        "mix" | "join" => {
            let tracks = tracks(arguments, folder)?;
            let file = target(op, "m4a")?;
            let mut command = out::ffmpeg_command(&ffmpeg);
            for (path, _, _) in &tracks {
                command.arg("-i").arg(path);
            }
            let mut filter = String::new();
            for (index, (_, volume, delay)) in tracks.iter().enumerate() {
                let delay_ms = if op == "mix" {
                    (delay * 1000.0) as u64
                } else {
                    0
                };
                filter.push_str(&format!(
                    "[{index}:a]aresample=48000,aformat=channel_layouts=stereo,volume={volume},adelay={delay_ms}:all=1[t{index}];"
                ));
            }
            let inputs: String = (0..tracks.len())
                .map(|index| format!("[t{index}]"))
                .collect();
            if op == "mix" {
                filter.push_str(&format!(
                    "{inputs}amix=inputs={}:duration=longest:normalize=0[out]",
                    tracks.len()
                ));
            } else {
                filter.push_str(&format!("{inputs}concat=n={}:v=0:a=1[out]", tracks.len()));
            }
            command
                .args(["-filter_complex", &filter, "-map", "[out]"])
                .arg(&file);
            out::run(command, FFMPEG_LIMIT, "ffmpeg")?;
            report["out"] = json!(file);
            report["tracks"] = json!(tracks.len());
            report["duration_s"] = json!(duration(&ffmpeg, &file));
        }
        _ => {
            let path = out::input(arguments, "path", folder, NAME)?;
            report["source"] = json!(path);
            let meta = probe::probe(&ffmpeg, &path)?;
            if meta.audio.is_none() {
                return Err(format!("{} has no sound", path.display()));
            }
            match op {
                "inspect" => {
                    let heard = sound::extract(&ffmpeg, &path, folder, Duration::from_secs(120))?;
                    let mut character = sound::character(&heard.samples, None);
                    character.loudness_lufs = heard.lufs;
                    character.loudness_range_lu = heard.loudness_range;
                    report["duration_s"] = json!((meta.duration_s * 10.0).round() / 10.0);
                    report["audio"] = json!(meta.audio);
                    report["bitrate_kbps"] = json!(meta.bitrate_kbps);
                    report["sound"] = json!(character);
                    if meta.width > 0 {
                        report["video"] = json!([meta.width, meta.height]);
                    }
                }
                "transcribe" => {
                    let stop = end
                        .unwrap_or(meta.duration_s)
                        .min(meta.duration_s.max(start));
                    let length = (stop - start).max(0.0);
                    progress("Reading the sound");
                    let deadline = started + TRANSCRIBE_BUDGET;
                    let heard = samples(&ffmpeg, &path, start, length, Duration::from_secs(120))?;
                    progress("Transcribing what is said");
                    let (lines, out_of_time) = sound::transcribe(&heard, deadline)?;
                    let lines: Vec<Line> = lines
                        .into_iter()
                        .map(|line| Line {
                            at_s: ((line.at_s + start) * 10.0).round() / 10.0,
                            text: line.text,
                        })
                        .collect();
                    let heard_until = start + heard.len() as f64 / f64::from(sound::RATE);
                    let text_file = folder.join(out::fresh_name(
                        folder,
                        &format!("{}-transcript", stem(&path)),
                        "txt",
                    ));
                    let (text, _) = super::captions::paragraphs(&lines, 30.0, usize::MAX);
                    let _ = std::fs::write(&text_file, &text);
                    let (shown, cut) = super::captions::paragraphs(&lines, 30.0, 40_000);
                    report["transcript"] = json!(if shown.is_empty() {
                        "(nothing said)".to_owned()
                    } else {
                        shown
                    });
                    report["transcript_file"] = json!(text_file);
                    report["from_s"] = json!(start);
                    report["to_s"] = json!((heard_until * 10.0).round() / 10.0);
                    if out_of_time || heard_until + 1.0 < stop || cut {
                        report["continue"] = json!(format!(
                            "Not all of it was transcribed in time; call again with start {}",
                            lines.last().map_or(start, |line| line.at_s).floor()
                        ));
                    }
                }
                "trim" | "convert" | "normalize" => {
                    let extension = match out::extension(&path).as_str() {
                        "mp3" => "mp3",
                        "wav" => "wav",
                        "flac" => "flac",
                        _ => "m4a",
                    };
                    let suffix = match op {
                        "trim" => "cut",
                        "normalize" => "normalized",
                        _ => "converted",
                    };
                    if op == "convert" && arguments["out"].as_str().is_none() {
                        return Err(
                            "convert needs out: a file name with the new extension (e.g. talk.mp3)"
                                .into(),
                        );
                    }
                    let file = target(&format!("{}-{suffix}", stem(&path)), extension)?;
                    let mut command = out::ffmpeg_command(&ffmpeg);
                    if op == "trim" {
                        if end.is_none() && start <= 0.0 {
                            return Err("trim needs start and/or end".into());
                        }
                        command.args(["-ss", &format!("{start:.3}")]);
                        if let Some(end) = end {
                            command.args(["-to", &format!("{end:.3}")]);
                        }
                    }
                    command.arg("-i").arg(&path).arg("-vn");
                    if op == "normalize" {
                        let target_lufs = arguments["target_lufs"]
                            .as_f64()
                            .unwrap_or(-16.0)
                            .clamp(-40.0, -5.0);
                        command.args([
                            "-af",
                            &format!("loudnorm=I={target_lufs}:TP=-1.5:LRA=11"),
                            "-ar",
                            "48000",
                        ]);
                        report["target_lufs"] = json!(target_lufs);
                    }
                    command.arg(&file);
                    progress("Processing the sound");
                    out::run(command, FFMPEG_LIMIT, "ffmpeg")?;
                    report["out"] = json!(file);
                    report["duration_s"] = json!(
                        duration(&ffmpeg, &file).map(|seconds| (seconds * 10.0).round() / 10.0)
                    );
                }
                other => return Err(format!("“{other}” isn't an op of {NAME}")),
            }
        }
    }
    report["took_s"] = json!(out::took(started));
    Ok(vec![json!({"type": "text", "text": report.to_string()})])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(content: &[Value]) -> Value {
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn sentences_are_cut_at_their_ends() {
        use crate::assistant::speech::neural::sentences;
        assert_eq!(
            sentences("Hallo Welt. Wie geht's? Gut!\nNeue Zeile ohne Punkt"),
            [
                "Hallo Welt.",
                "Wie geht's?",
                "Gut!",
                "Neue Zeile ohne Punkt"
            ]
        );
        assert_eq!(sentences("v1.2 ist da."), ["v1.2 ist da."]);
        assert!(sentences(" \n ").is_empty());
    }

    /// Cuts, converts, normalizes, mixes, and joins sound files made by
    /// ffmpeg (files only, no sound device).
    #[test]
    fn sound_files_are_cut_converted_and_mixed() {
        let Ok(ffmpeg) = tools::ffmpeg(&mut |_| {}) else {
            eprintln!("no ffmpeg: skipped");
            return;
        };
        let folder = std::env::temp_dir().join(format!("hex-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for (name, frequency) in [("tone.wav", 440), ("low.wav", 220)] {
            let mut command = out::ffmpeg_command(&ffmpeg);
            command
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency={frequency}:duration=6"),
                    "-af",
                    "volume=0.1",
                ])
                .arg(folder.join(name));
            out::run(command, Duration::from_secs(30), "ffmpeg").unwrap();
        }
        let started = Instant::now();
        let call = |arguments: Value| report(&call(&arguments, &folder, &mut |_| {}).unwrap());
        let inspected = call(json!({"op": "inspect", "path": "tone.wav"}));
        assert_eq!(inspected["duration_s"], 6.0);
        assert_eq!(inspected["sound"]["kind"], "sound");
        let cut = call(
            json!({"op": "trim", "path": "tone.wav", "start": "0:01", "end": 3.5, "out": "cut.mp3"}),
        );
        let length = cut["duration_s"].as_f64().unwrap();
        assert!((length - 2.5).abs() < 0.15, "{length}");
        let louder = call(json!({"op": "normalize", "path": "tone.wav"}));
        let level = call(json!({"op": "inspect", "path": louder["out"]}))["sound"]["loudness_lufs"]
            .as_f64()
            .unwrap();
        assert!((level - -16.0).abs() < 2.0, "{level}");
        let mixed = call(
            json!({"op": "mix", "tracks": ["tone.wav", {"path": "low.wav", "volume": 0.5, "delay": 2}]}),
        );
        assert!(
            (mixed["duration_s"].as_f64().unwrap() - 8.0).abs() < 0.2,
            "{mixed}"
        );
        let joined =
            call(json!({"op": "join", "tracks": ["tone.wav", "low.wav"], "out": "both.wav"}));
        assert!(
            (joined["duration_s"].as_f64().unwrap() - 12.0).abs() < 0.2,
            "{joined}"
        );
        assert!(
            super::call(
                &json!({"op": "convert", "path": "tone.wav"}),
                &folder,
                &mut |_| {}
            )
            .is_err()
        );
        assert!(
            super::call(
                &json!({"op": "mix", "tracks": ["tone.wav"]}),
                &folder,
                &mut |_| {}
            )
            .is_err()
        );
        let heard = samples(
            &ffmpeg,
            &folder.join("tone.wav"),
            2.0,
            3.0,
            Duration::from_secs(20),
        )
        .unwrap();
        assert_eq!(heard.len(), 3 * sound::RATE as usize);
        println!("audio ops: {:.1} s", started.elapsed().as_secs_f64());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Speaks into a file with Agent-Desk's voice (when installed) or
    /// `say`, then reads it: no sound device either way.
    #[cfg(target_os = "macos")]
    #[test]
    fn text_is_spoken_into_a_file() {
        let folder = std::env::temp_dir().join(format!("hex-speak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let started = Instant::now();
        let spoken = report(
            &call(
                &json!({"op": "speak", "text": "Hallo. Das ist ein kurzer Test.", "language": "de", "out": "hallo.m4a"}),
                &folder,
                &mut |_| {},
            )
            .unwrap(),
        );
        println!("speak: {:.1} s, {spoken}", started.elapsed().as_secs_f64());
        let seconds = spoken["duration_s"].as_f64().unwrap();
        assert!((1.0..15.0).contains(&seconds), "{spoken}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Transcribes HEX_AUDIO_SOURCE through the running Agent-Desk:
    /// `HEX_AUDIO_SOURCE=… cargo test --bin agent-desk media::audio::tests::transcribe_a_given_file -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the running app's transcription"]
    fn transcribe_a_given_file() {
        let source = std::env::var("HEX_AUDIO_SOURCE").expect("HEX_AUDIO_SOURCE");
        let folder = std::env::temp_dir();
        let started = Instant::now();
        let content = call(
            &json!({"op": "transcribe", "path": source}),
            &folder,
            &mut |step| println!("{step}"),
        )
        .unwrap();
        println!(
            "took {:.1} s\n{}",
            started.elapsed().as_secs_f64(),
            content[0]["text"]
        );
    }
}
