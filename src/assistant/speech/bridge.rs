//! The Swift bridge to AVSpeechSynthesizer (`src/speech_output.swift`): it
//! writes sentences to audio, which comes back through [`start`]'s callback
//! and plays through [`super::output`].

use std::ffi::{CStr, CString, c_char};

use super::voices::{Listed, Quality};

/// A sentence's audio: id, samples (mono, 48 kHz), count, and whether that
/// was the last of it.
pub type Deliver = extern "C" fn(u64, *const f32, usize, bool);

unsafe extern "C" {
    fn hex_speech_output_start(callback: Deliver);
    fn hex_speech_output_main_state(timeout_ms: i32) -> i32;
    fn hex_speech_output_speak(id: u64, text: *const c_char, voice: *const c_char, rate: f32);
    fn hex_speech_output_stop();
    fn hex_speech_output_voices() -> *mut c_char;
    fn hex_speech_output_free_string(string: *mut c_char);
}

/// Registers where sentence audio goes. Once, before [`speak`].
pub fn start(deliver: Deliver) {
    // SAFETY: Stores a function pointer that lives as long as the program.
    unsafe { hex_speech_output_start(deliver) }
}

/// Whether the main queue, where the synthesizer calls back, is served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Main {
    /// The app's run loop answers.
    Answers,
    /// Asked from a main thread that runs no loop (`agent-desk dev …`): each
    /// sentence runs it while being written.
    Pumped,
    /// Nobody answered in time (a blocked main thread, as in tests).
    Silent,
}

pub fn main_state(timeout_ms: i32) -> Main {
    // SAFETY: Takes and returns plain integers.
    match unsafe { hex_speech_output_main_state(timeout_ms) } {
        1 => Main::Answers,
        2 => Main::Pumped,
        _ => Main::Silent,
    }
}

/// Writes `text` with `voice` (an identifier; `None` for the system's) at
/// `rate` (AVSpeechUtterance's 0..1, 0.5 normal), after the sentences
/// before it.
pub fn speak(id: u64, text: &str, voice: Option<&str>, rate: f32) {
    let Ok(text) = CString::new(text.replace('\0', " ")) else {
        return;
    };
    let voice = voice.and_then(|voice| CString::new(voice).ok());
    let voice = voice
        .as_ref()
        .map_or(std::ptr::null(), |voice| voice.as_ptr());
    // SAFETY: Both strings are valid C strings for the call; Swift copies
    // them.
    unsafe { hex_speech_output_speak(id, text.as_ptr(), voice, rate) }
}

/// Drops queued sentences and stops writing the current one.
pub fn stop() {
    // SAFETY: Takes nothing; dispatches to the main queue.
    unsafe { hex_speech_output_stop() }
}

#[derive(serde::Deserialize)]
struct BridgeVoice {
    id: String,
    name: String,
    language: String,
    quality: u8,
}

/// The installed voices AVSpeechSynthesizer can read with.
pub fn voices() -> Vec<Listed> {
    // SAFETY: Returns an owned C string or null.
    let string = unsafe { hex_speech_output_voices() };
    if string.is_null() {
        return Vec::new();
    }
    // SAFETY: The bridge returned a valid C string it allocated.
    let json = unsafe { CStr::from_ptr(string) }.to_bytes().to_vec();
    // SAFETY: Freed once, by the allocator that made it.
    unsafe { hex_speech_output_free_string(string) };
    parse(&json)
}

fn parse(json: &[u8]) -> Vec<Listed> {
    let voices: Vec<BridgeVoice> = serde_json::from_slice(json).unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the voice list");
        Vec::new()
    });
    voices
        .into_iter()
        .map(|voice| Listed {
            id: voice.id,
            name: voice.name,
            locale: voice.language,
            quality: match voice.quality {
                4 => Quality::Personal,
                3 => Quality::Premium,
                2 => Quality::Enhanced,
                _ => Quality::Default,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bridges_voice_list_is_read() {
        let voices = parse(
            br#"[{"id":"com.apple.voice.premium.de-DE.Petra","name":"Petra","language":"de-DE","quality":3},
                 {"id":"p","name":"Felix","language":"en-US","quality":4}]"#,
        );
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].quality, Quality::Premium);
        assert_eq!(voices[0].locale, "de-DE");
        assert_eq!(voices[1].quality, Quality::Personal);
        assert!(parse(b"nonsense").is_empty());
    }
}
