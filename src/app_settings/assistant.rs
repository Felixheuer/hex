//! The assistant's settings (macOS): its switch, the Action shortcut, the
//! agent that answers, and how far it may act. The switch is
//! `assistant.enabled`, on by default; the Anthropic API key lives in the
//! Keychain, never here.

use super::*;

/// The agent that answers and acts. Hex finds which are installed
/// (`assistant::backends::detect`); each task runs as its own process or
/// session of it, with agent-desktop as its desktop tool.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantBackend {
    /// `claude -p`, signed in with the person's own Claude account.
    #[default]
    ClaudeCode,
    /// `codex exec`, signed in with the person's ChatGPT account.
    Codex,
    /// The OpenCode service already running on this computer.
    OpenCode,
    /// Anthropic's Messages API with a key from the Keychain.
    ClaudeApi,
}

/// The assistant's options.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AssistantSettings {
    /// The assistant's switch (macOS): off, the Action shortcut, the
    /// overlays, the scheduler, and its menu bar items are inert. On by
    /// default.
    pub enabled: bool,
    /// Hold and say anything: the assistant answers, acts in the window in
    /// front, or starts a background job. Double-tap shows or hides the task
    /// list; a tap then a hold forces a new Background task. Right Command
    /// by default.
    pub action_hotkey: Option<HotkeyBinding>,
    pub backend: AssistantBackend,
    /// Each backend's model; `None` is that agent's own default.
    pub claude_code_model: Option<String>,
    pub codex_model: Option<String>,
    /// OpenCode's `provider/model`.
    pub opencode_model: Option<String>,
    /// OpenCode with the person's own API key for this provider
    /// (`assistant::backends::providers`), the key in the Keychain; `None`
    /// uses OpenCode's own sign-ins.
    pub opencode_provider: Option<String>,
    pub claude_api_model: String,
    /// The Here lane's model on Claude Code and the Claude API, for fast
    /// spoken answers; `None` uses the model above. Codex and OpenCode keep
    /// their own model.
    pub here_model: Option<String>,
    /// Answers to a quick question are read aloud.
    pub speak_answers: bool,
    /// The macOS voice name; `None` is the system voice.
    pub voice: Option<String>,
    /// Tasks may operate apps through agent-desktop.
    pub control_desktop: bool,
    /// Tasks may also move the real pointer and press real keys
    /// (agent-desktop `--headed`); off keeps every action in the background.
    pub physical_input: bool,
    /// A quick question sends a screenshot of the screen with it.
    pub include_screenshot: bool,
    /// A hold within this many seconds of a quick answer continues that
    /// conversation; 0 always starts a new one. A Background task always starts one.
    pub follow_up_seconds: u32,
    /// At most this many tasks run at once; later ones wait their turn.
    pub max_parallel: u32,
    /// The setup sheet (agents found, agent-desktop, permissions) was finished.
    pub setup_done: bool,
    /// Stops every task at once, and the speech: Control-Option-Escape by
    /// default. Never Command-Option-Escape, which is macOS's Force Quit.
    pub stop_all_hotkey: Option<HotkeyBinding>,
    /// A Here task may move the real pointer and press real keys in the
    /// window in front (under a "Hands off" pill); off keeps it headless too.
    pub here_physical_input: bool,
    /// Web work runs in agent-browser's own hidden browsers, one session per
    /// task, instead of in the person's browser windows.
    pub background_browser: bool,
    /// Apps no task may read or operate (by name or bundle id).
    pub blocked_apps: Vec<String>,
    /// Background tasks may also operate the window in front of the
    /// person, as the ChatGPT app's agent does; off keeps that window theirs.
    pub background_front_window: bool,
    /// A task running longer than this many minutes is stopped; 0 never.
    pub task_timeout_minutes: u32,
    /// A task that hits its agent's usage limit waits and retries.
    pub retry_on_limit: bool,
    /// Experimental: windows Background tasks open move to a Space of their
    /// own, out of the person's way.
    pub agent_space: bool,
    /// The display new Background windows go to, by agent-desktop id ("5")
    /// or name ("M27Q"); `None` is the one the person isn't working on
    /// (with two or more displays; with one, windows stay where they open).
    pub agent_display: Option<String>,
    /// How freely tasks take consequential actions (send, delete, pay,
    /// share, publish): ask each time, only what the request named, or yolo.
    pub autonomy: crate::assistant::autonomy::Autonomy,
    /// A short note about the person (business, people, tone) every task
    /// reads; edited in Settings and by `hex_remember`. Stored beside the
    /// conversations, not here; this only turns it on.
    pub memory: bool,
    /// After a spoken answer, Hex keeps listening this many seconds for a
    /// follow-up without the shortcut; 0 turns conversation mode off.
    pub conversation_seconds: u32,
    /// Speaking while an answer is read aloud interrupts it (needs
    /// headphones or Hex's echo suppression).
    pub barge_in: bool,
    /// macOS notifications when a job finishes, fails, or asks something.
    pub notifications: bool,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            action_hotkey: Some(HotkeyBinding::action_default()),
            backend: AssistantBackend::ClaudeCode,
            claude_code_model: None,
            codex_model: None,
            opencode_model: None,
            opencode_provider: None,
            claude_api_model: "claude-sonnet-5-5".into(),
            here_model: Some("claude-haiku-5-5".into()),
            speak_answers: true,
            voice: None,
            control_desktop: true,
            physical_input: false,
            include_screenshot: true,
            follow_up_seconds: 120,
            max_parallel: 8,
            setup_done: false,
            stop_all_hotkey: Some(HotkeyBinding::stop_all_default()),
            here_physical_input: true,
            background_browser: true,
            // macOS's password apps, then every OS's password managers.
            blocked_apps: [
                "Passwords",
                "Keychain Access",
                "1Password",
                "Bitwarden",
                "KeePassXC",
                "KeePass",
                "Seahorse",
            ]
            .map(String::from)
            .to_vec(),
            background_front_window: false,
            task_timeout_minutes: 30,
            retry_on_limit: true,
            agent_space: false,
            agent_display: None,
            autonomy: crate::assistant::autonomy::Autonomy::Smart,
            memory: true,
            conversation_seconds: 8,
            barge_in: true,
            notifications: true,
        }
    }
}

impl HotkeyBinding {
    /// Stop all tasks' default: Control-Option-Escape on macOS,
    /// Ctrl+Alt+Escape on Windows, and Ctrl+Alt+Q on Linux, where KDE's
    /// Ctrl+Alt+Escape kills a window and the desktop portal refuses Escape.
    pub fn stop_all_default() -> Self {
        #[cfg(target_os = "macos")]
        let binding = Self {
            modifiers: HotkeyModifiers {
                control: Some(ModifierSide::Either),
                option: Some(ModifierSide::Either),
                ..Default::default()
            },
            key: Some(HotkeyKey {
                code: 53,
                label: "Esc".into(),
            }),
        };
        #[cfg(target_os = "windows")]
        let binding = Self::chord(HotkeyModifiers::control_option(), "escape");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let binding = Self::chord(HotkeyModifiers::control_option(), "q");
        binding
    }

    /// macOS's Force Quit Applications, Command-Option-Escape, which no
    /// shortcut of the assistant's may take.
    pub fn force_quit() -> Self {
        Self {
            modifiers: HotkeyModifiers {
                command: Some(ModifierSide::Either),
                option: Some(ModifierSide::Either),
                ..Default::default()
            },
            key: Some(HotkeyKey {
                code: 53,
                label: "Esc".into(),
            }),
        }
    }

    /// The Action shortcut's default: hold right Command alone on macOS,
    /// right Ctrl alone on Windows (right Alt is AltGr on many layouts, and
    /// right Win opens Start), and Ctrl+Alt+A on Linux, whose shortcuts
    /// always need a key (X11 grabs and the desktop portal take chords).
    pub fn action_default() -> Self {
        #[cfg(target_os = "macos")]
        let binding = Self {
            modifiers: HotkeyModifiers {
                command: Some(ModifierSide::Right),
                ..Default::default()
            },
            key: None,
        };
        #[cfg(target_os = "windows")]
        let binding = Self {
            modifiers: HotkeyModifiers {
                control: Some(ModifierSide::Right),
                ..Default::default()
            },
            key: None,
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let binding = Self::chord(HotkeyModifiers::control_option(), "a");
        binding
    }
}

impl AppSettings {
    /// The assistant's Stop all tasks shortcut while it takes effect: the
    /// assistant on, bound, and sharing no key with another active shortcut
    /// or Force Quit. Otherwise Hex matches and swallows nothing for it.
    pub fn active_stop_all_hotkey(&self) -> Option<&HotkeyBinding> {
        if !crate::assistant::enabled(self) {
            return None;
        }
        self.assistant
            .stop_all_hotkey
            .as_ref()
            .filter(|binding| binding.key.is_some())
            .filter(|binding| !self.hotkey_in_use(HotkeySlot::StopAll, binding))
    }
}

/// Whether the assistant is ready to ask (`assistant::ready`) as the
/// settings were last applied, for the listener's Action shortcut; false
/// until settings load.
pub fn applied_assistant_ready() -> bool {
    HOTKEYS
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .settings
        .as_deref()
        .is_some_and(crate::assistant::ready)
}

/// The assistant's settings as last applied, and whether it is on, for
/// the task runner's threads; `None` until settings are applied (a `hex`
/// command that never loads them reads the file instead).
pub fn assistant_runtime() -> Option<(bool, AssistantSettings)> {
    ASSISTANT_RUNTIME
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub(super) fn apply_assistant(settings: &AppSettings) {
    *ASSISTANT_RUNTIME
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = Some((
        crate::assistant::enabled(settings),
        settings.assistant.clone(),
    ));
}

static ASSISTANT_RUNTIME: OnceLock<RwLock<Option<(bool, AssistantSettings)>>> = OnceLock::new();
