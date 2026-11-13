//! `preferences`: the handful of ordinary switches and choices that used to
//! be Labs features (`crate::features`). Labs is gone: every former lab
//! either graduated (always on, no switch), became one of these settings, or
//! was removed. A settings file from before keeps loading: its `labs` object
//! is read once and carried here ([`AppSettings::migrate_retired_labs`]).

use super::*;

/// What happens to a finished dictation's text.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterDictation {
    /// Paste it, then put the previous clipboard back.
    #[default]
    Paste,
    /// Paste it and leave it on the clipboard.
    PasteAndKeep,
    /// Only put it on the clipboard (macOS and Windows).
    CopyOnly,
}

impl AfterDictation {
    /// Every choice, in the order the window shows them.
    pub const ALL: [Self; 3] = [Self::Paste, Self::PasteAndKeep, Self::CopyOnly];

    /// The English label the window shows.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Paste => "Paste",
            Self::PasteAndKeep => "Paste and keep on clipboard",
            Self::CopyOnly => "Copy only",
        }
    }
}

/// Caps Lock as a dictation key (macOS): off, or on with its light steady or
/// blinking slowly while Hex records.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapsLockMode {
    #[default]
    Off,
    /// Dictates; the light stays on for the whole recording.
    Steady,
    /// Dictates; the light blinks every 700 ms while recording.
    SlowBlink,
}

impl CapsLockMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Steady, Self::SlowBlink];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Steady => "Dictate, light steady",
            Self::SlowBlink => "Dictate, light blinks",
        }
    }
}

/// The main window's appearance: the OS's, or one forced on it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceTheme {
    /// Follow the OS.
    #[default]
    System,
    /// Always light, whatever the OS shows.
    Light,
    /// Always dark, whatever the OS shows.
    Dark,
}

impl AppearanceTheme {
    /// Every choice, in the order the window shows them.
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// The English label the window shows.
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// The idle times the speech model may sit unused before it unloads, in
/// minutes.
pub const UNLOAD_IDLE_MINUTES: [u32; 4] = [5, 15, 30, 60];

/// How long the microphone may stay open after a dictation, in seconds:
/// 30 s, 1, 3, 5, and 30 minutes.
pub const KEEP_MICROPHONE_OPEN_SECONDS: [u32; 5] = [30, 60, 180, 300, 1_800];

/// The former Labs features that became ordinary settings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    /// Provisional words beside the HUD while dictating.
    pub live_text: bool,
    /// Provisional words in the focused text field as a Wayland input-method
    /// preedit, with the final text typed there (Linux).
    pub live_typing: bool,
    /// Paste, paste and keep, or copy only.
    pub after_dictation: AfterDictation,
    /// Spoken "new line", "open quote", "comma", "hashtag" and the like
    /// become line breaks, quotes, and symbols.
    pub spoken_formatting: bool,
    /// Filler words such as "um" and "uh" leave dictations.
    pub remove_filler_words: bool,
    /// Minutes the speech model may sit unused before it unloads; `None`
    /// keeps it loaded. One of [`UNLOAD_IDLE_MINUTES`].
    #[serde(deserialize_with = "deserialize_unload_minutes")]
    pub unload_model_after_minutes: Option<u32>,
    /// Seconds the microphone stays open after a dictation while Release
    /// microphone while idle is on; `None` releases it at once. One of
    /// [`KEEP_MICROPHONE_OPEN_SECONDS`].
    #[serde(deserialize_with = "deserialize_keep_microphone_seconds")]
    pub keep_microphone_open_seconds: Option<u32>,
    /// Caps Lock as a dictation key (macOS).
    pub caps_lock: CapsLockMode,
    /// The main window's appearance.
    pub appearance: AppearanceTheme,
    /// The menu-bar icon hides while nothing records (macOS).
    pub hide_menu_bar_icon: bool,
    /// ⌘Q closes the window instead of quitting, so the assistant and
    /// dictation keep running (macOS).
    pub quit_closes_window: bool,
    /// "While dictating" applies only while the default output is speakers.
    pub speakers_only: bool,
    /// Dictations saved to History keep their audio, for Transcribe again.
    pub keep_recordings: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            live_text: true,
            live_typing: false,
            after_dictation: AfterDictation::Paste,
            spoken_formatting: true,
            remove_filler_words: true,
            unload_model_after_minutes: None,
            keep_microphone_open_seconds: None,
            caps_lock: CapsLockMode::Off,
            appearance: AppearanceTheme::System,
            hide_menu_bar_icon: false,
            quit_closes_window: true,
            speakers_only: false,
            keep_recordings: false,
        }
    }
}

impl Preferences {
    /// How long a capture's microphone keeps open once it ends; `None` turns
    /// it off, as does a value outside [`KEEP_MICROPHONE_OPEN_SECONDS`].
    pub fn keep_microphone_open_for(&self) -> Option<std::time::Duration> {
        self.keep_microphone_open_seconds
            .filter(|seconds| KEEP_MICROPHONE_OPEN_SECONDS.contains(seconds))
            .map(|seconds| std::time::Duration::from_secs(u64::from(seconds)))
    }

    /// How long the speech model may sit unused before it unloads.
    pub fn unload_model_after(&self) -> Option<std::time::Duration> {
        self.unload_model_after_minutes
            .filter(|minutes| UNLOAD_IDLE_MINUTES.contains(minutes))
            .map(|minutes| std::time::Duration::from_secs(u64::from(minutes) * 60))
    }
}

fn deserialize_choice<'de, D>(deserializer: D, choices: &[u32]) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<u32>::deserialize(deserializer)?;
    match value {
        Some(value) if !choices.contains(&value) => Err(serde::de::Error::custom(format!(
            "expected one of {}",
            choices
                .iter()
                .map(|choice| choice.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        value => Ok(value),
    }
}

fn deserialize_unload_minutes<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_choice(deserializer, &UNLOAD_IDLE_MINUTES)
}

fn deserialize_keep_microphone_seconds<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_choice(deserializer, &KEEP_MICROPHONE_OPEN_SECONDS)
}

/// The retired `labs` object, read only to carry its values into
/// [`Preferences`] and the shortcuts once. Every switch defaulted to off.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct RetiredLabs {
    live_text: bool,
    live_typing: bool,
    keep_on_clipboard: bool,
    copy_only: bool,
    spoken_formatting: bool,
    spoken_symbols: bool,
    filler_words: bool,
    unload_idle: bool,
    unload_idle_minutes: Option<u32>,
    keep_microphone_open: bool,
    keep_microphone_open_seconds: Option<u32>,
    caps_lock_dictation: bool,
    caps_lock_light: Option<String>,
    appearance: bool,
    appearance_theme: AppearanceTheme,
    hide_menu_bar_icon: bool,
    quit_closes_window: bool,
    speakers_only: bool,
    keep_recordings: bool,
    second_dictation_shortcut: bool,
    more_dictation_shortcuts: bool,
    typed_text: bool,
    terminal_paste_shortcut: bool,
}

impl RetiredLabs {
    /// The preferences these switches meant.
    pub(super) fn preferences(&self) -> Preferences {
        let after_dictation = if self.copy_only {
            AfterDictation::CopyOnly
        } else if self.keep_on_clipboard {
            AfterDictation::PasteAndKeep
        } else {
            AfterDictation::Paste
        };
        let caps_lock = match (self.caps_lock_dictation, self.caps_lock_light.as_deref()) {
            (false, _) => CapsLockMode::Off,
            (true, Some("slow_blink")) => CapsLockMode::SlowBlink,
            (true, _) => CapsLockMode::Steady,
        };
        Preferences {
            live_text: self.live_text,
            live_typing: self.live_typing,
            after_dictation,
            spoken_formatting: self.spoken_formatting || self.spoken_symbols,
            remove_filler_words: self.filler_words,
            unload_model_after_minutes: self
                .unload_idle
                .then(|| self.unload_idle_minutes.unwrap_or(15))
                .filter(|minutes| UNLOAD_IDLE_MINUTES.contains(minutes)),
            keep_microphone_open_seconds: self
                .keep_microphone_open
                .then(|| self.keep_microphone_open_seconds.unwrap_or(60))
                .filter(|seconds| KEEP_MICROPHONE_OPEN_SECONDS.contains(seconds)),
            caps_lock,
            appearance: if self.appearance {
                self.appearance_theme
            } else {
                AppearanceTheme::System
            },
            hide_menu_bar_icon: self.hide_menu_bar_icon,
            quit_closes_window: self.quit_closes_window,
            speakers_only: self.speakers_only,
            keep_recordings: self.keep_recordings,
        }
    }
}

impl AppSettings {
    /// Carries a settings file's retired `labs` object into `preferences`
    /// once, and unbinds the shortcuts whose lab was off: those shortcuts
    /// now always take effect, so one saved while its lab was off must not
    /// start working by surprise. On macOS the second dictation shortcut
    /// becomes the first of the extra dictation shortcuts.
    pub(super) fn migrate_retired_labs(&mut self, labs: &RetiredLabs) {
        self.preferences = labs.preferences();
        if !labs.second_dictation_shortcut {
            self.second_dictation_hotkey = None;
        }
        if !labs.more_dictation_shortcuts {
            self.extra_dictation_hotkeys.clear();
        }
        if !labs.typed_text {
            self.typed_text_hotkey = None;
        }
        if !labs.terminal_paste_shortcut {
            self.terminal_paste_hotkey = None;
        }
        self.fold_second_dictation_shortcut();
    }

    /// macOS has the extra dictation shortcuts, so a second dictation
    /// shortcut moves into them (when there is room) rather than being a
    /// second way to set the same thing.
    pub(super) fn fold_second_dictation_shortcut(&mut self) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let Some(binding) = self.second_dictation_hotkey.take() else {
            return;
        };
        if self.extra_dictation_hotkeys.len() < MAX_EXTRA_DICTATION_SHORTCUTS {
            self.extra_dictation_hotkeys.insert(
                0,
                ExtraDictationHotkey {
                    binding: Some(binding),
                    mode: ExtraDictationMode::default(),
                },
            );
        }
    }

    /// The saved preferences, read without loading, normalizing, or writing
    /// anything else; a file from before carries its `labs` object.
    pub fn saved_preferences() -> Preferences {
        path()
            .ok()
            .and_then(|path| fs::read(path).ok())
            .map(|data| preferences_in(&data))
            .unwrap_or_default()
    }
}

/// The preferences a settings file's bytes hold: `preferences`, or what an
/// older file's `labs` meant.
pub(super) fn preferences_in(data: &[u8]) -> Preferences {
    #[derive(Deserialize)]
    struct File {
        preferences: Option<Preferences>,
        labs: Option<RetiredLabs>,
    }
    match serde_json::from_slice::<File>(data) {
        Ok(File {
            preferences: Some(preferences),
            ..
        }) => preferences,
        Ok(File {
            labs: Some(labs), ..
        }) => labs.preferences(),
        _ => Preferences::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_without_labs_or_preferences_gets_the_defaults() {
        assert_eq!(preferences_in(b"{}"), Preferences::default());
        assert!(Preferences::default().live_text);
        assert!(Preferences::default().quit_closes_window);
    }

    #[test]
    fn retired_labs_carry_into_preferences() {
        let data = br#"{"labs":{"live_text":true,"copy_only":true,"spoken_symbols":true,
            "unload_idle":true,"unload_idle_minutes":30,"keep_microphone_open":true,
            "keep_microphone_open_seconds":180,"caps_lock_dictation":true,
            "caps_lock_light":"slow_blink","appearance":true,"appearance_theme":"dark",
            "pause_menu":true,"groups":{"models":true}}}"#;
        let preferences = preferences_in(data);
        assert!(preferences.live_text);
        assert_eq!(preferences.after_dictation, AfterDictation::CopyOnly);
        assert!(preferences.spoken_formatting);
        assert!(!preferences.remove_filler_words);
        assert_eq!(preferences.unload_model_after_minutes, Some(30));
        assert_eq!(preferences.keep_microphone_open_seconds, Some(180));
        assert_eq!(preferences.caps_lock, CapsLockMode::SlowBlink);
        assert_eq!(preferences.appearance, AppearanceTheme::Dark);
        assert!(!preferences.quit_closes_window);
    }

    #[test]
    fn a_theme_without_its_switch_follows_the_system() {
        let data = br#"{"labs":{"appearance":false,"appearance_theme":"dark"}}"#;
        assert_eq!(preferences_in(data).appearance, AppearanceTheme::System);
    }

    #[test]
    fn preferences_win_over_a_stale_labs_object() {
        let data = br#"{"preferences":{"live_text":false},"labs":{"live_text":true}}"#;
        assert!(!preferences_in(data).live_text);
    }

    #[test]
    fn an_unknown_duration_fails_to_load_and_falls_back() {
        let error = serde_json::from_str::<Preferences>(r#"{"keep_microphone_open_seconds":7}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected one of"), "{error}");
        let preferences: Preferences =
            serde_json::from_str(r#"{"unload_model_after_minutes":15}"#).unwrap();
        assert_eq!(
            preferences.unload_model_after(),
            Some(std::time::Duration::from_secs(900))
        );
    }
}
