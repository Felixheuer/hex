//! macOS's fallback voices: `/usr/bin/say` writes each sentence to a WAV
//! file ([`super::writer`] plays it), where the speech engine can't answer.

use std::sync::OnceLock;

use super::playback::RATE;
use super::voices::{self, Voice};

const SAY: &str = "/usr/bin/say";
/// Words a minute: a little faster than `say`'s 175, as the speech engine
/// reads.
const WORDS_PER_MINUTE: &str = "190";

/// Nothing to set up on the writer's thread.
pub fn prepare_thread() {}

/// Nothing outlives a stop: ending `say` ends its sentence.
pub fn stopped() {}

/// `say -v '?'`, read once.
pub fn voices() -> Vec<Voice> {
    static VOICES: OnceLock<Vec<Voice>> = OnceLock::new();
    VOICES
        .get_or_init(|| {
            let output = crate::desktop::background_process::command(SAY)
                .args(["-v", "?"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            match output {
                Ok(output) => voices::parse_voices(&String::from_utf8_lossy(&output.stdout)),
                Err(error) => {
                    tracing::warn!(%error, "could not list the voices");
                    Vec::new()
                }
            }
        })
        .clone()
}

/// Has `say` write `text` with `voice` to a file and reads it back.
pub fn write(text: &str, voice: Option<&Voice>, generation: u64) -> Option<Vec<f32>> {
    let file = super::writer::temp_file("wav");
    let mut command = crate::desktop::background_process::command(SAY);
    if let Some(voice) = voice {
        command.args(["-v", &voice.id]);
    }
    command
        .args(["-r", WORDS_PER_MINUTE])
        .args(["--file-format=WAVE", &format!("--data-format=LEF32@{RATE}")])
        .arg("-o")
        .arg(&file)
        // The text goes in on stdin, so one that starts with "-" is never
        // an option.
        .args(["-f", "-"]);
    let status = super::writer::run(command, text, generation).map(|(status, _)| status);
    super::writer::take_file(&file, status)
}
