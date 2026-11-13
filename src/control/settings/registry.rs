//! The `hex settings` key registry: every key, its kind, the retired names,
//! and the capability each key needs.

use crate::app_settings::HotkeySlot;

#[derive(Clone, Copy, Debug)]
pub(super) enum Kind {
    Bool,
    Number {
        min: f64,
        max: f64,
    },
    #[cfg_attr(not(hex_capture), allow(dead_code))]
    Integer {
        min: u64,
        max: u64,
    },
    Choice(&'static [&'static str]),
    Text,
    Shortcut(HotkeySlot),
    Path,
    Json,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Number { .. } => "number",
            Self::Integer { .. } => "integer",
            Self::Choice(_) => "choice",
            Self::Text => "text",
            Self::Shortcut(_) => "shortcut",
            Self::Path => "path",
            Self::Json => "json",
        }
    }
}

pub(super) struct Key {
    pub(super) key: &'static str,
    pub(super) kind: Kind,
    /// `none` clears it.
    pub(super) optional: bool,
    /// Why it can't be set here, if it can't.
    pub(super) read_only: Option<&'static str>,
    pub(super) about: &'static str,
}

const fn key(key: &'static str, kind: Kind, about: &'static str) -> Key {
    Key {
        key,
        kind,
        optional: false,
        read_only: None,
        about,
    }
}

const fn optional(key: &'static str, kind: Kind, about: &'static str) -> Key {
    Key {
        key,
        kind,
        optional: true,
        read_only: None,
        about,
    }
}

const fn read_only(key: &'static str, kind: Kind, about: &'static str, why: &'static str) -> Key {
    Key {
        key,
        kind,
        optional: false,
        read_only: Some(why),
        about,
    }
}

const UNIT: Kind = Kind::Number { min: 0.0, max: 1.0 };
const MODEL_CHANGE: &str = "change the transcription model with `hex model use`";

/// Choice keys whose values are numbers in settings.json.
pub(super) const NUMERIC_CHOICES: [&str; 3] = [
    "meetings.calendar.lead_minutes",
    "preferences.unload_model_after_minutes",
    "preferences.keep_microphone_open_seconds",
];

pub(super) const KEYS: &[Key] = &[
    key(
        "keyboard_model_switching.enabled",
        Kind::Bool,
        "Switch the model when the keyboard input source changes (macOS, Windows, X11)",
    ),
    key(
        "keyboard_model_switching.mappings",
        Kind::Json,
        "Input-source IDs mapped to a model and language; absent sources don't switch (macOS, Windows, X11)",
    ),
    key(
        "release_microphone_while_idle",
        Kind::Bool,
        "Release the microphone between dictations",
    ),
    key(
        "sound_effects",
        Kind::Bool,
        "Play recording and error tones",
    ),
    key(
        "sound_effect_volume",
        UNIT,
        "Tone volume from 0 to 1; 0 turns tones off",
    ),
    key(
        "sound_theme",
        Kind::Choice(&[
            "signature",
            "classic",
            "click",
            "soft",
            "wood",
            "analog",
            "thock",
        ]),
        "Which sounds play; classic, Hex's original tones, is the default",
    ),
    optional(
        "microphone",
        Kind::Text,
        "Microphone by name; system-default (or none) follows the system's default input",
    ),
    key(
        "avoid_bluetooth_microphone",
        Kind::Bool,
        "System default records from the built-in or another wired microphone, not a Bluetooth headset's",
    ),
    key(
        "recording_audio_behavior",
        Kind::Choice(&["mute", "pause_media", "do_nothing", "duck"]),
        "What other audio does while you dictate",
    ),
    key(
        "recording_duck_volume",
        UNIT,
        "Fraction of original output volume while lowering audio, from 0 to 1",
    ),
    key(
        "pause_browser_media",
        Kind::Bool,
        "Also pause a playing browser tab while dictating (macOS, with Pause media)",
    ),
    key(
        "double_tap_lock",
        Kind::Bool,
        "Double-tap the dictation shortcut to keep recording hands-free",
    ),
    key(
        "double_tap_only",
        Kind::Bool,
        "Only a double tap starts dictation (needs double_tap_lock and a key chord)",
    ),
    key(
        "shortcut_mode",
        Kind::Choice(&["hold", "toggle"]),
        "Hold the shortcut while speaking, or press once to start and again to finish",
    ),
    key(
        "dictation_hotkey",
        Kind::Shortcut(HotkeySlot::Dictation),
        "The dictation shortcut",
    ),
    optional(
        "paste_last_hotkey",
        Kind::Shortcut(HotkeySlot::PasteLast),
        "The paste-last-dictation shortcut; none turns it off",
    ),
    key(
        "cancel_hotkey",
        Kind::Shortcut(HotkeySlot::Cancel),
        "Toggle mode's cancel shortcut, Escape by default: discards the recording, \
         nothing pasted or saved. Hex takes the key only while a toggled recording runs",
    ),
    optional(
        "raw_dictation_hotkey",
        Kind::Shortcut(HotkeySlot::RawDictation),
        "Dictate without processing: dictates and pastes the words as heard, skipping \
         the Output steps; none (the default) turns it off",
    ),
    optional(
        "second_dictation_hotkey",
        Kind::Shortcut(HotkeySlot::SecondDictation),
        "Another shortcut that dictates like dictation_hotkey, such as a key on an \
         external keyboard (Linux and Windows, which need a key; macOS uses \
         extra_dictation_hotkeys); none (the default) turns it off",
    ),
    key(
        "extra_dictation_hotkeys",
        Kind::Json,
        "Up to four more dictation shortcuts as a JSON array of \
         {\"binding\": ..., \"mode\": ...} rows, each with its own activation mode \
         (macOS); an empty array (the default) has none",
    ),
    optional(
        "typed_text_hotkey",
        Kind::Shortcut(HotkeySlot::TypedText),
        "The shortcut that runs the focused app's Output steps on the text already \
         typed in the focused field and replaces it (Linux); none (the default) turns it off",
    ),
    optional(
        "terminal_paste_hotkey",
        Kind::Shortcut(HotkeySlot::TerminalPaste),
        "Another shortcut that dictates like dictation_hotkey and always pastes with \
         Ctrl+Shift+V (Windows, Linux); none (the default) turns it off",
    ),
    key(
        "screenshots.hotkey",
        Kind::Shortcut(HotkeySlot::Screenshot),
        "Retired: the screenshot shortcut of the removed Screenshots feature (kept in the file, unused)",
    ),
    optional(
        "screenshots.record_hotkey",
        Kind::Shortcut(HotkeySlot::ScreenshotRecord),
        "Retired: the screen recording shortcut of the removed Screenshots feature (kept in the file, unused)",
    ),
    optional(
        "screenshots.folder",
        Kind::Path,
        "Retired: the folder of the removed Screenshots feature (kept in the file, unused)",
    ),
    key(
        "assistant.enabled",
        Kind::Bool,
        "The assistant: hold the Action shortcut to ask by voice, double-tap for your tasks; \
         Chats lists the conversations; on by default (macOS)",
    ),
    optional(
        "assistant.action_hotkey",
        Kind::Shortcut(HotkeySlot::Action),
        "Hold and say anything, and the assistant answers, acts, or starts a \
         background job; double-tap shows the task list; right command by default; none \
         turns it off (macOS)",
    ),
    key(
        "assistant.backend",
        Kind::Choice(&["claude_code", "codex", "open_code", "claude_api"]),
        "The agent that answers and acts: claude_code, codex, open_code, or claude_api \
         (an Anthropic key in the Keychain) (macOS)",
    ),
    optional(
        "assistant.claude_code_model",
        Kind::Text,
        "The model Claude Code uses for the assistant; none is its default (macOS)",
    ),
    optional(
        "assistant.here_model",
        Kind::Text,
        "The model for Here-lane answers on Claude Code and the Claude API; none uses \
         the backend's model (macOS)",
    ),
    optional(
        "assistant.here_effort",
        Kind::Choice(&["low", "medium", "high", "xhigh", "max"]),
        "How hard a Here-lane turn on Claude Code thinks; none is the model's default \
         (macOS)",
    ),
    key(
        "assistant.escalate_after_tools",
        Kind::Integer { min: 0, max: 100 },
        "A Here-lane turn on the fast model goes on with Claude Code's own model after \
         this many tool calls, or when it fails; 0 never (macOS)",
    ),
    key(
        "assistant.warm_minutes",
        Kind::Integer { min: 0, max: 120 },
        "Claude Code stays started for the next hold this many minutes after a quick \
         answer; 0 only for the follow-up window (macOS)",
    ),
    optional(
        "assistant.codex_model",
        Kind::Text,
        "The model Codex uses for the assistant; none is its default (macOS)",
    ),
    optional(
        "assistant.opencode_model",
        Kind::Text,
        "OpenCode's provider/model for the assistant; none is its default (macOS)",
    ),
    optional(
        "assistant.opencode_provider",
        Kind::Text,
        "The provider whose API key (saved in the Keychain) OpenCode uses for the assistant, such as deepseek or openrouter; none uses OpenCode's own sign-ins (macOS)",
    ),
    key(
        "assistant.claude_api_model",
        Kind::Text,
        "The Claude model the API backend uses, claude-sonnet-5-5 by default (macOS)",
    ),
    key(
        "assistant.speak_answers",
        Kind::Bool,
        "The assistant reads quick answers aloud (macOS)",
    ),
    optional(
        "assistant.voice",
        Kind::Text,
        "The macOS voice that reads answers; none uses the system voice (macOS)",
    ),
    key(
        "assistant.control_desktop",
        Kind::Bool,
        "Assistant tasks may operate apps through agent-desktop (macOS)",
    ),
    key(
        "assistant.physical_input",
        Kind::Bool,
        "Assistant tasks may also move the real pointer and press real keys (macOS)",
    ),
    key(
        "assistant.include_screenshot",
        Kind::Bool,
        "A quick question sends a screenshot of the screen with it; off by default, \
         the agent looks with its desktop tools when it needs to (macOS)",
    ),
    key(
        "assistant.follow_up_seconds",
        Kind::Integer { min: 0, max: 600 },
        "A hold this soon after a quick answer continues that conversation; 0 always \
         starts a new one (macOS)",
    ),
    key(
        "assistant.max_parallel",
        Kind::Integer { min: 1, max: 50 },
        "At most this many assistant tasks run at once; later ones wait (macOS)",
    ),
    optional(
        "assistant.stop_all_hotkey",
        Kind::Shortcut(HotkeySlot::StopAll),
        "Stops every assistant task and its speech at once; control+option+escape by \
         default; none turns it off (macOS)",
    ),
    key(
        "assistant.here_physical_input",
        Kind::Bool,
        "A Here task may use the real pointer and keys in the window in front (macOS)",
    ),
    key(
        "assistant.background_front_window",
        Kind::Bool,
        "Background assistant tasks may also operate the window in front, like the ChatGPT app (macOS)",
    ),
    key(
        "assistant.projects_folder",
        Kind::Text,
        "Where the assistant's coding jobs create their projects; ~/dev by default (macOS)",
    ),
    key(
        "assistant.keep_awake",
        Kind::Bool,
        "The computer doesn't sleep while a background assistant task works (the display may)",
    ),
    key(
        "assistant.keep_display_awake",
        Kind::Bool,
        "With assistant.keep_awake, the display stays on too",
    ),
    key(
        "assistant.phone_enabled",
        Kind::Bool,
        "While you're away, questions, approvals and results go to your iPhone by iMessage, and your replies come back (macOS)",
    ),
    optional(
        "assistant.phone_handle",
        Kind::Text,
        "Your own phone number or Apple ID for assistant.phone_enabled; nothing else is read or written (macOS)",
    ),
    key(
        "assistant.phone_away_minutes",
        Kind::Integer { min: 0, max: 240 },
        "Away after this many minutes without input, or when the screen locks; 0 only when you say so (macOS)",
    ),
    key(
        "assistant.browser_signed_in_by_default",
        Kind::Bool,
        "Every new assistant browser starts with the person's accounts (macOS)",
    ),
    optional(
        "assistant.browser_import_profile",
        Kind::Text,
        "The Chrome profile whose logins Hex's browsers take over daily, such as Default; none takes none (macOS)",
    ),
    key(
        "assistant.background_browser",
        Kind::Bool,
        "Web work runs in agent-browser's own hidden browsers, one per task (macOS)",
    ),
    key(
        "assistant.blocked_apps",
        Kind::Json,
        "Apps no assistant task may read or operate, by name or bundle id (macOS)",
    ),
    key(
        "assistant.task_timeout_minutes",
        Kind::Integer { min: 0, max: 1440 },
        "An assistant question or chat running longer than this is stopped; 0 never (macOS)",
    ),
    key(
        "assistant.job_budget_minutes",
        Kind::Integer { min: 0, max: 10080 },
        "A background assistant job works until its goal is met, through restarts, for at most this many minutes (default 7 days); 0 never stops it (macOS)",
    ),
    key(
        "assistant.retry_on_limit",
        Kind::Bool,
        "A task that hits its agent's usage limit waits and retries (macOS)",
    ),
    key(
        "assistant.agent_space",
        Kind::Bool,
        "Experimental; windows Background tasks open move to a Space of their own (macOS)",
    ),
    optional(
        "assistant.agent_display",
        Kind::Text,
        "The display new Background task windows move to, by agent-desktop id or name; \
         none is the display the person isn't working on (macOS)",
    ),
    key(
        "assistant.autonomy",
        Kind::Choice(&["ask", "smart", "yolo"]),
        "How freely assistant tasks send, delete, pay, share, or publish: ask each \
         time, smart (only what your request named, the default), or yolo (macOS)",
    ),
    key(
        "assistant.memory",
        Kind::Bool,
        "Assistant tasks read your note about you (business, people, tone) (macOS)",
    ),
    key(
        "assistant.conversation_seconds",
        Kind::Integer { min: 0, max: 60 },
        "After a spoken answer Hex listens this long for a follow-up without the \
         shortcut; 0 turns conversation mode off (macOS)",
    ),
    key(
        "assistant.barge_in",
        Kind::Bool,
        "Speaking while an answer is read aloud interrupts it (macOS)",
    ),
    key(
        "assistant.notifications",
        Kind::Bool,
        "MacOS notifications when an assistant job finishes, fails, or asks (macOS)",
    ),
    key(
        "assistant.setup_done",
        Kind::Bool,
        "The assistant's setup sheet was finished (macOS)",
    ),
    key(
        "assistant.setup_version",
        Kind::Integer { min: 0, max: 1000 },
        "The assistant setup version finished; a lower one brings back the steps a release added",
    ),
    key(
        "show_dock_icon",
        Kind::Bool,
        "Keep Hex in the Dock when Settings is closed (macOS)",
    ),
    read_only(
        "transcription.model",
        Kind::Text,
        "The dictation model",
        MODEL_CHANGE,
    ),
    read_only(
        "transcription.language",
        Kind::Text,
        "The dictation language",
        MODEL_CHANGE,
    ),
    read_only(
        "transcription_recents",
        Kind::Json,
        "Recently used models, each with its language",
        "Hex records these as you switch models",
    ),
    key(
        "vocabulary_boost",
        Kind::Choice(&["off", "light", "normal", "strong"]),
        "How readily transcription swaps heard words for a vocabulary term",
    ),
    key(
        "dictation_processing.default_mode",
        Kind::Json,
        "The Global mode: replacements, transformations, and rewriting",
    ),
    key(
        "dictation_processing.modes",
        Kind::Json,
        "Modes for specific apps and websites, as a JSON array",
    ),
    key(
        "history_retention",
        Kind::Choice(&["off", "day", "week", "month", "forever"]),
        "How long dictation history is kept",
    ),
    key(
        "preferences.live_text",
        Kind::Bool,
        "Provisional words beside the HUD while dictating (on by default)",
    ),
    key(
        "preferences.live_typing",
        Kind::Bool,
        "Provisional words in the focused text field as an underlined input-method preedit, and the final text typed there instead of pasted (Linux Wayland)",
    ),
    key(
        "preferences.after_dictation",
        Kind::Choice(&["paste", "paste_and_keep", "copy_only"]),
        "What happens to a dictation: paste it and restore the clipboard (the default), paste and leave it on the clipboard, or only copy it (copy_only: macOS and Windows)",
    ),
    key(
        "preferences.spoken_formatting",
        Kind::Bool,
        "Spoken \"new line\", \"open quote\", \"comma\", \"hashtag\" and similar words become line breaks, quotes, and symbols (on by default)",
    ),
    key(
        "preferences.remove_filler_words",
        Kind::Bool,
        "Remove filler words such as um and uh from dictations (on by default)",
    ),
    optional(
        "preferences.unload_model_after_minutes",
        Kind::Choice(&["5", "15", "30", "60"]),
        "Minutes the speech model may sit unused before it unloads, freeing its memory; none (the default) keeps it loaded",
    ),
    optional(
        "preferences.keep_microphone_open_seconds",
        Kind::Choice(&["30", "60", "180", "300", "1800"]),
        "With release_microphone_while_idle on, seconds the microphone stays open after a dictation so the next one starts at once; none (the default) releases it at once",
    ),
    key(
        "preferences.caps_lock",
        Kind::Choice(&["off", "steady", "slow_blink"]),
        "Caps Lock as a dictation key; its light stays on or blinks slowly while Hex records (macOS)",
    ),
    key(
        "preferences.appearance",
        Kind::Choice(&["system", "light", "dark"]),
        "The window's appearance: follow the system (the default), or always light or dark",
    ),
    key(
        "preferences.hide_menu_bar_icon",
        Kind::Bool,
        "Hide Hex's menu bar icon while nothing records; shortcuts keep working (macOS)",
    ),
    key(
        "preferences.quit_closes_window",
        Kind::Bool,
        "⌘Q closes the window instead of quitting Hex, so the assistant and dictation keep running; the menu bar icon still quits (macOS, on by default)",
    ),
    key(
        "preferences.speakers_only",
        Kind::Bool,
        "While dictating applies only while the default output is speakers; headphones are left alone",
    ),
    key(
        "preferences.keep_recordings",
        Kind::Bool,
        "Dictations saved to History keep their audio, so History can transcribe them again with another model",
    ),
    key(
        "meetings.enabled",
        Kind::Bool,
        "Meetings: call detection, recording, and transcripts",
    ),
    optional(
        "meetings.output_folder",
        Kind::Path,
        "Folder for meeting Markdown; none uses ~/Documents/Hex Meetings",
    ),
    key(
        "meetings.summaries",
        SUMMARIES,
        "Who titles and summarizes meetings: off, mac (Apple Intelligence, nothing leaves this Mac), or opencode (sends the transcript to the OpenCode model)",
    ),
    optional(
        "meetings.opencode_model",
        Kind::Text,
        "The OpenCode provider/model meeting notes use when meetings.summaries is opencode; none lets Hex pick a fast model",
    ),
    key(
        "meetings.show_recording_indicator",
        Kind::Bool,
        "Show the floating recording indicator during meetings",
    ),
    key(
        "meetings.recording_indicator_corner",
        Kind::Choice(&["top_right", "top_left", "bottom_right", "bottom_left"]),
        "Screen corner of the meeting recording indicator",
    ),
    optional(
        "meetings.retention",
        Kind::Choice(&["day", "week", "month", "forever"]),
        "How long finished meetings are kept before they move to the Trash; none keeps new meetings 24 hours and older ones until you choose",
    ),
    optional(
        "meetings.audio_retention",
        Kind::Choice(&["day", "week", "month", "forever"]),
        "How long finished meetings keep their audio; none follows meetings.retention",
    ),
    key(
        "meetings.providers",
        Kind::Json,
        "Per-app call choices, as a JSON array of {key, label, policy: ask|always_record|never_offer}",
    ),
    read_only(
        "meetings.calendar.connected",
        Kind::Bool,
        "A calendar is connected for meeting reminders",
        "connect a calendar from Settings › Meetings, which asks macOS for access",
    ),
    key(
        "meetings.calendar.excluded_calendars",
        Kind::Json,
        "Calendars left out of reminders, as a JSON array of EventKit identifiers",
    ),
    key(
        "meetings.calendar.remind",
        Kind::Bool,
        "Remind before calendar meetings",
    ),
    key(
        "meetings.calendar.lead_minutes",
        Kind::Choice(&["0", "1", "2", "5"]),
        "Minutes before a meeting its reminder shows",
    ),
    key(
        "experimental.hex_microphone",
        Kind::Bool,
        "Developer: the Voice Filter and its Hex Microphone virtual input (dev builds)",
    ),
    key(
        "experimental.hex_microphone_mode",
        Kind::Choice(&[
            "normal",
            "muffled",
            "quiet",
            "muted",
            "chipmunk",
            "robot",
            "walkie",
            "underwater",
            "far",
        ]),
        "What Hex Microphone does to your voice while you dictate",
    ),
    key(
        "experimental.hex_microphone_comfort_noise",
        Kind::Bool,
        "Hex Microphone fills silence with soft noise",
    ),
    key(
        "experimental.hex_microphone_strength",
        UNIT,
        "How strongly Hex Microphone applies its filter, from 0 to 1",
    ),
    key(
        "enable_on_launch",
        Kind::Bool,
        "Start listening when Hex opens (Windows and Linux)",
    ),
    key(
        "paste_with_shift",
        Kind::Bool,
        "Paste with Ctrl-Shift-V for terminals (Linux and Windows)",
    ),
    key(
        "gnome_keyboard_shortcuts",
        Kind::Bool,
        "Hex's own GNOME custom keyboard shortcuts, for GNOME without a shortcuts portal (Linux)",
    ),
    key(
        "share_dictation_shortcut",
        Kind::Bool,
        "Let other apps see the dictation shortcut too (Windows)",
    ),
    optional(
        "ui_language",
        Kind::Choice(&[
            "en", "pl", "zh", "zh-Hant", "ja", "de", "es", "uk", "pt", "fr", "ru",
        ]),
        "Interface language; none follows the system",
    ),
    key(
        "transcription_device",
        Kind::Choice(&["auto", "gpu", "cpu"]),
        "Where the speech model runs: auto (a GPU unless it failed before), gpu, or cpu (Linux and Windows)",
    ),
    key(
        "indicator_position",
        Kind::Choice(&["top", "bottom", "hidden"]),
        "Where the dictation HUD appears",
    ),
    optional(
        "updates.channel",
        Kind::Choice(&["stable", "dev"]),
        "Update channel; none follows the build (dev builds use dev, others stable)",
    ),
];

/// Captures exist only in builds with the `capture` build option.
#[cfg(hex_capture)]
pub(super) const CAPTURE_KEYS: &[Key] = &[
    key(
        "capture.enabled",
        Kind::Bool,
        "Annotate: record the screen while you draw and talk (on by default)",
    ),
    key(
        "capture.hotkey",
        Kind::Shortcut(HotkeySlot::Capture),
        "The Annotate shortcut: press to start recording and again to finish",
    ),
    optional(
        "capture.replay_hotkey",
        Kind::Shortcut(HotkeySlot::CaptureReplay),
        "Retired: the shortcut of Annotate's removed 15 s replay buffer (kept in the file, unused)",
    ),
    key(
        "capture.transcriber",
        Kind::Choice(&["parakeet", "apple"]),
        "What transcribes a capture's narration: Hex's loaded model or Apple's",
    ),
    optional(
        "capture.folder",
        Kind::Path,
        "Folder for new captures; none uses Scrawl's captures folder",
    ),
    optional(
        "capture.media_days",
        RETENTION_DAYS,
        "Days a capture keeps its video and audio; none keeps them",
    ),
    optional(
        "capture.bundle_days",
        RETENTION_DAYS,
        "Days a capture is kept; none keeps it",
    ),
    key(
        "capture.capsule_flow",
        Kind::Choice(&["list", "toast", "menu_bar", "cursor", "notch"]),
        "How the capture capsule offers where to send a capture (prototypes): a list, a \
         one-line toast, the menu bar, a card at the pointer, or a notch",
    ),
    key(
        "capture.routing",
        Kind::Json,
        "Where captures go among OpenCode sessions: Scrawl's router settings (autoSend, sites, apps, smartRouting, …) as a JSON object",
    ),
    key(
        "capture.show_overlays_in_recordings",
        Kind::Bool,
        "Let screen recordings, captures included, show Hex's capsules, ink, and HUD (for demos)",
    ),
];
#[cfg(not(hex_capture))]
pub(super) const CAPTURE_KEYS: &[Key] = &[];
#[cfg(hex_capture)]
const RETENTION_DAYS: Kind = Kind::Integer { min: 1, max: 3650 };

#[cfg(target_os = "macos")]
const SUMMARIES: Kind = Kind::Choice(&["off", "mac", "opencode"]);
#[cfg(not(target_os = "macos"))]
const SUMMARIES: Kind = Kind::Choice(&["off", "opencode"]);

/// Keys that were replaced, and what replaced them.
pub(super) const RETIRED: &[(&str, &str)] = &[
    (
        "meetings.summarize_with_apple_intelligence",
        "meetings.summaries mac",
    ),
    (
        "meetings.summarize_with_opencode",
        "meetings.summaries opencode",
    ),
];

/// Settings that act only where Hex has a capability (by its
/// `rune_ui::capabilities` name), the same one that shows their row in the
/// window. The first matching prefix decides.
pub(super) const NEEDS: &[(&str, &str)] = &[
    ("meetings.calendar.", "calendar"),
    ("meetings.", "meetings"),
    ("experimental.hex_microphone", "fun"),
    ("typed_text_hotkey", "typed_text"),
    ("show_dock_icon", "dock_icon"),
    ("preferences.hide_menu_bar_icon", "dock_icon"),
    ("preferences.quit_closes_window", "dock_icon"),
    ("keyboard_model_switching.", "keyboard_model_switching"),
    ("enable_on_launch", "listener_control"),
    ("share_dictation_shortcut", "share_shortcut"),
];

/// The capability `key` needs, if any.
pub(super) fn needs(key: &str) -> Option<&'static str> {
    NEEDS
        .iter()
        .find(|(prefix, _)| key == *prefix || key.starts_with(prefix))
        .map(|(_, capability)| *capability)
}
