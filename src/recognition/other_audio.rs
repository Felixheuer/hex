//! Whether another app plays audio now (music, a video, a call), for
//! conversation mode: on speakers its microphone would hear it, and the
//! speech detector passes sung vocals, so a song's lyrics would become a
//! question. Read from Core Audio's metadata at most every
//! [`CHECK_EVERY`], never from audio.
//!
//! macOS 14.2 and later tell per process, so Hex (and the `say` it starts)
//! is left out. Earlier systems only tell whether the default output runs
//! at all, which Hex's own reading also does: there it counts only while
//! Hex isn't speaking.

use std::time::{Duration, Instant};

/// How often Core Audio is asked again.
const CHECK_EVERY: Duration = Duration::from_millis(500);

/// The last answer, and when it was read.
#[derive(Default)]
pub(super) struct OtherAudio {
    checked: Option<Instant>,
    playing: bool,
}

impl OtherAudio {
    /// Whether another app plays audio at `now`; `hex_speaking`, whether
    /// Hex reads aloud (for the fallback that can't tell Hex apart).
    pub(super) fn playing(&mut self, now: Instant, hex_speaking: bool) -> bool {
        if !due(self.checked, now) {
            return self.playing;
        }
        self.checked = Some(now);
        self.playing = decide(read(), hex_speaking);
        self.playing
    }
}

/// Whether a read is due: never read, or the last is [`CHECK_EVERY`] old.
fn due(checked: Option<Instant>, now: Instant) -> bool {
    checked.is_none_or(|checked| now.saturating_duration_since(checked) >= CHECK_EVERY)
}

/// What Core Audio said.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Read {
    /// Per process, Hex left out.
    Others(bool),
    /// Only the default output device, Hex included.
    Device(bool),
    /// Nothing could be read.
    Unknown,
}

fn decide(read: Read, hex_speaking: bool) -> bool {
    match read {
        Read::Others(playing) => playing,
        Read::Device(running) => running && !hex_speaking,
        Read::Unknown => false,
    }
}

#[cfg(target_os = "macos")]
fn read() -> Read {
    if let Ok(playing) = crate::microphone_activity::others_running_output() {
        return Read::Others(playing);
    }
    crate::microphone_activity::default_output_running().map_or(Read::Unknown, Read::Device)
}

#[cfg(not(target_os = "macos"))]
fn read() -> Read {
    Read::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_audio_is_asked_at_most_every_half_second() {
        let start = Instant::now();
        assert!(due(None, start));
        assert!(!due(Some(start), start + Duration::from_millis(499)));
        assert!(due(Some(start), start + Duration::from_millis(500)));
    }

    #[test]
    fn the_device_fallback_counts_only_while_hex_is_quiet() {
        assert!(decide(Read::Others(true), true));
        assert!(!decide(Read::Others(false), false));
        assert!(decide(Read::Device(true), false));
        assert!(!decide(Read::Device(true), true), "Hex's own reading");
        assert!(!decide(Read::Unknown, false));
    }
}
