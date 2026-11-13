//! What used to be Labs. There is no Labs anymore: no pane, no switches, no
//! `hex labs`. Each former lab was decided once (see
//! `docs/features/labs.md`):
//!
//! - **Graduated**: always on where the OS has it, with no switch. Most
//!   gates below return a constant.
//! - **A setting**: an ordinary choice in `AppSettings::preferences`
//!   (`crate::app_settings::Preferences`), shown in the window where it
//!   belongs. These gates read this process's copy of it.
//! - **Removed**: the gate is `false` and the window offers nothing.
//!
//! Gate a feature where its work starts (the worker, thread, or capture that
//! does it), not only where its UI shows.

use std::sync::RwLock;

use crate::app_settings::{
    AfterDictation, AppSettings, AppearanceTheme, CapsLockMode, Preferences,
};

/// This process's preferences: unknown until settings load or one is first
/// read.
static PREFERENCES: RwLock<Option<Preferences>> = RwLock::new(None);

fn preferences() -> Preferences {
    if let Some(preferences) = *PREFERENCES.read().unwrap_or_else(|e| e.into_inner()) {
        return preferences;
    }
    let saved = AppSettings::saved_preferences();
    let mut slot = PREFERENCES.write().unwrap_or_else(|e| e.into_inner());
    *slot.get_or_insert(saved)
}

/// Records the preferences the loaded or saved settings hold.
pub(crate) fn apply(preferences: &Preferences) {
    *PREFERENCES.write().unwrap_or_else(|e| e.into_inner()) = Some(*preferences);
    // Pause from the menu bar is always offered now; nothing to reconcile
    // beyond keeping a pause the person chose.
    crate::shortcut_pause::reconcile(pause_menu());
}

/// Whether the main window draws dark: the OS's appearance, unless
/// Appearance names one. Resolved for each view, so System keeps following
/// the OS live. Overlay windows (the HUD, capsules, live text) keep following
/// the OS themselves.
pub fn window_dark(preferences: &Preferences, system_dark: bool) -> bool {
    match preferences.appearance {
        AppearanceTheme::System => system_dark,
        AppearanceTheme::Light => false,
        AppearanceTheme::Dark => true,
    }
}

/// Logs the preferences to process.log; call once logging is up in a
/// long-lived Hex.
pub fn log_launch() {
    let preferences = preferences();
    tracing::info!(?preferences, "preferences");
}

// ---------------------------------------------------------------------------
// Settings (`preferences`)

/// Live text: provisional words beside the HUD (read as each dictation starts).
pub fn live_text() -> bool {
    preferences().live_text
}

/// Live typing: provisional words in the focused text field as an
/// input-method preedit on Wayland (Linux; read as each capture starts).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn live_typing() -> bool {
    cfg!(target_os = "linux") && preferences().live_typing
}

/// A pasted dictation stays on the clipboard rather than the previous
/// clipboard being restored (read as each paste commits).
pub fn keep_on_clipboard() -> bool {
    preferences().after_dictation == AfterDictation::PasteAndKeep
}

/// A dictation goes to the clipboard and is never pasted (macOS and Windows;
/// read as each dictation's output commits).
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
pub fn copy_only() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
        && preferences().after_dictation == AfterDictation::CopyOnly
}

/// Spoken formatting phrases become line breaks and quotation marks.
pub fn spoken_formatting() -> bool {
    preferences().spoken_formatting
}

/// Spoken punctuation and symbol words become characters (one setting with
/// spoken formatting).
pub fn spoken_symbols() -> bool {
    preferences().spoken_formatting
}

/// Filler words such as "um" and "uh" leave dictations.
pub fn filler_words() -> bool {
    preferences().remove_filler_words
}

/// History keeps each dictation's 16 kHz audio for Transcribe again.
pub fn keep_recordings() -> bool {
    preferences().keep_recordings
}

/// The menu-bar icon hides while nothing records (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn hide_menu_bar_icon() -> bool {
    cfg!(target_os = "macos") && preferences().hide_menu_bar_icon
}

/// ⌘Q closes the window instead of quitting Hex (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn quit_closes_window() -> bool {
    cfg!(target_os = "macos") && preferences().quit_closes_window
}

/// "While dictating" applies only while the default output is speakers.
pub fn speakers_only() -> bool {
    preferences().speakers_only
}

/// A physical Caps Lock press starts and stops dictation (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn caps_lock_dictation() -> bool {
    cfg!(target_os = "macos") && preferences().caps_lock != CapsLockMode::Off
}

/// Whether the Caps Lock light blinks slowly while recording (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn caps_lock_slow_blink() -> bool {
    cfg!(target_os = "macos") && preferences().caps_lock == CapsLockMode::SlowBlink
}

/// How long the speech model may sit unused before the engine unloads it.
pub fn unload_idle() -> Option<std::time::Duration> {
    preferences().unload_model_after()
}

/// How long a capture's microphone keeps open after it ends, while Release
/// microphone while idle is on. `None` releases it at once.
pub fn microphone_linger() -> Option<std::time::Duration> {
    preferences().keep_microphone_open_for()
}

// ---------------------------------------------------------------------------
// Graduated: always on where the OS has it

/// A failed dictation's explanation beside the HUD.
pub fn failure_notices() -> bool {
    true
}

/// Live chapters while a meeting records.
pub fn live_chapters() -> bool {
    true
}

/// Transcription biased toward the vocabulary (Parakeet).
pub fn vocabulary_biasing() -> bool {
    true
}

/// Mouse buttons may be recorded as shortcuts (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mouse_shortcuts() -> bool {
    cfg!(target_os = "macos")
}

/// A dictation pastes into the app its capture started in (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn return_to_app() -> bool {
    cfg!(target_os = "macos")
}

/// A pasted dictation gets one leading space when the character before the
/// cursor asks for one (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn smart_spacing() -> bool {
    cfg!(target_os = "macos")
}

/// The shortcut recorder keeps each modifier's side in a modifier-only chord
/// (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sided_modifier_chords() -> bool {
    cfg!(target_os = "macos")
}

/// In Toggle mode, a press held past the hold threshold finishes its capture
/// when it lifts (macOS, Linux).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn tap_or_hold() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

/// Double-tap mode accepts a shortcut without a key (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn double_tap_modifiers() -> bool {
    cfg!(target_os = "macos")
}

/// Parakeet Ultra is in the model catalog.
pub fn parakeet_ultra() -> bool {
    true
}

/// History's summary and `hex history stats`.
pub fn history_stats() -> bool {
    true
}

/// History selects several dictations at once.
pub fn history_multi_select() -> bool {
    true
}

/// The menu bar's Copy Recent Dictation submenu (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn recent_dictations() -> bool {
    cfg!(target_os = "macos")
}

/// An automatic meeting recording stops once its call ends (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn stop_when_call_ends() -> bool {
    cfg!(target_os = "macos")
}

/// Calls in more apps and on more websites are named and offered.
pub fn more_call_apps() -> bool {
    true
}

/// Meeting capsules show in the saved corner and can be dragged (macOS).
pub fn movable_meeting_capsules() -> bool {
    cfg!(target_os = "macos")
}

/// Meeting titles and summaries read the vocabulary.
pub fn meeting_notes_vocabulary() -> bool {
    true
}

/// The HUD grows out of the notch on a screen that has one (macOS). Off:
/// the speech bubble shows at its own place at the top, as it always did
/// (the notch variant was a Lab few had on; turning it on for everyone hid
/// the bubble the person relies on).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn notch_indicator() -> bool {
    false
}

/// A paste on GNOME restores the clipboard through XWayland (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn gnome_clipboard_restore() -> bool {
    cfg!(target_os = "linux")
}

/// A shortcut transforms the text already typed (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn typed_text() -> bool {
    cfg!(target_os = "linux")
}

/// The meeting capsules float on XWayland under GNOME (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn gnome_meeting_capsules() -> bool {
    cfg!(target_os = "linux")
}

/// Annotate's capsule and ink float on XWayland under GNOME (Linux).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn gnome_annotate() -> bool {
    cfg!(target_os = "linux")
}

/// The local API's `GET /meetings/events` serves finished meetings.
pub fn meeting_finished_event() -> bool {
    true
}

/// Audio files may be imported as meetings (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn meeting_import() -> bool {
    cfg!(target_os = "macos")
}

/// Notes or a rename rename an exported meeting's Markdown to its title.
pub fn meeting_descriptive_filenames() -> bool {
    true
}

/// A meeting's notes are written to their own file beside the transcript.
pub fn meeting_notes_file() -> bool {
    true
}

/// The menu bar offers Pause Hex (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pause_menu() -> bool {
    cfg!(target_os = "macos")
}

/// `hex dictation start --print` (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn print_start() -> bool {
    cfg!(target_os = "macos")
}

/// An OpenCode service in WSL is used when no Windows one answers (Windows).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn opencode_in_wsl() -> bool {
    cfg!(target_os = "windows")
}

/// The local API's `process=global`.
pub fn api_processing() -> bool {
    true
}

/// The local API's `timestamps=segment|word`.
pub fn api_timestamps() -> bool {
    true
}

/// Finished meetings drop their WAV tracks on `meetings.audio_retention`.
pub fn expire_meeting_audio() -> bool {
    true
}

/// A dictation mode's speech model and language follow the foreground app
/// (macOS and Windows).
pub fn app_model_switching() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

// ---------------------------------------------------------------------------
// Removed

/// The developer playground (story browser): no switch and no sidebar item;
/// dev builds open it with ⌘⇧D or `hex window story`. Stable builds never
/// have it.
pub fn playground() -> bool {
    crate::dev_build::is_dev_build() && !cfg!(test)
}

/// Whole-clip transcription skipping long pauses was removed.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn skip_pauses() -> bool {
    false
}

/// The screenshot shortcut and its Captures pane were removed: the assistant
/// sees the screen itself, and macOS has its own screenshot keys.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn screenshots() -> bool {
    false
}

/// Annotate's rolling 15 s replay buffer was removed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn annotate_replay() -> bool {
    false
}

/// The Voice Filter (Hex Microphone) was a developer tool; only an installed
/// driver's Uninstall remains.
pub fn voice_filter() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_forces_the_window_or_follows_the_system() {
        let mut preferences = Preferences::default();
        assert!(window_dark(&preferences, true));
        assert!(!window_dark(&preferences, false));
        preferences.appearance = AppearanceTheme::Light;
        assert!(!window_dark(&preferences, true));
        preferences.appearance = AppearanceTheme::Dark;
        assert!(window_dark(&preferences, false));
    }
}
