//! Reading answers aloud with the macOS voice, on this computer.
//!
//! OWNER: hands (with `computer`).

/// Starts reading `text` aloud with `voice` (`None`: the system voice),
/// stopping anything already being read. Returns at once.
pub fn speak(text: &str, voice: Option<&str>) {
    let _ = (text, voice);
}

/// Stops reading aloud.
pub fn stop() {}

/// Whether something is being read now.
pub fn speaking() -> bool {
    false
}

/// The installed voices' names, for Settings.
pub fn voices() -> Vec<String> {
    Vec::new()
}
