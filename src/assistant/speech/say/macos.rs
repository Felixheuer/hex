//! `/usr/bin/say`, macOS's speech program: the fallback when
//! AVSpeechSynthesizer's main queue doesn't answer.

use std::path::Path;

use super::super::playback::RATE;
use super::super::voices::{self, Voice};
use super::{Job, Plan};

const SAY: &str = "/usr/bin/say";
/// Words a minute: a little faster than `say`'s 175, as the speech engine
/// reads.
const WORDS_PER_MINUTE: &str = "190";

/// `say` writing `job` as 32-bit float WAV at the output's rate, its text
/// on stdin so one that starts with "-" is never an option.
pub fn plan(job: &Job, file: &Path) -> Plan {
    let mut command = crate::desktop::background_process::command(SAY);
    if let Some(voice) = &job.voice {
        command.args(["-v", voice]);
    }
    command
        .args(["-r", WORDS_PER_MINUTE])
        .args(["--file-format=WAVE", &format!("--data-format=LEF32@{RATE}")])
        .arg("-o")
        .arg(file)
        .args(["-f", "-"]);
    Plan::Command {
        command,
        stdin: Some(job.text.clone()),
    }
}

/// `say -v '?'`.
pub fn voices() -> Vec<Voice> {
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
}
