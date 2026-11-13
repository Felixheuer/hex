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
    /// How hard a Here turn on Claude Code thinks (`--effort`: low, medium,
    /// high, xhigh, max); `None` is the model's own default. A job keeps
    /// its model's default.
    pub here_effort: Option<String>,
    /// A Here turn on the fast model (`here_model`) that has made this many
    /// tool calls without finishing goes on with the agent's own model
    /// (`claude_code_model`) in the same session, as does one that failed;
    /// 0 never hands over. Claude Code only.
    pub escalate_after_tools: u32,
    /// Claude Code is kept started and ready for the next hold this many
    /// minutes after the last Here answer (its process waits idle and costs
    /// no tokens until the next question); 0 keeps it only for the
    /// follow-up window.
    pub warm_minutes: u32,
    /// Answers to a quick question are read aloud.
    pub speak_answers: bool,
    /// The macOS voice name; `None` is the system voice.
    pub voice: Option<String>,
    /// Tasks may operate apps through agent-desktop.
    pub control_desktop: bool,
    /// Real input, one switch per lane (`here_physical_input` is the
    /// other): Background tasks may also move the real pointer and press
    /// real keys (agent-desktop `--headed`); off, the default, keeps every
    /// action of theirs in the background. Was `physical_input`.
    #[serde(alias = "physical_input")]
    pub background_physical_input: bool,
    /// A quick question sends a screenshot of the screen with it. Off by
    /// default: the agent looks with its desktop tools when it needs to,
    /// so the fast path encodes and sends no image.
    pub include_screenshot: bool,
    /// The longer of the two follow-up windows (`conversation_seconds` is
    /// the short, hands-free one; Settings shows both in one row): a hold
    /// within this many seconds of a quick answer continues that
    /// conversation; 0 always starts a new one. A Background task always
    /// starts one.
    pub follow_up_seconds: u32,
    /// At most this many tasks run at once; later ones wait their turn.
    pub max_parallel: u32,
    /// The setup sheet (agents found, agent-desktop, permissions) was finished.
    pub setup_done: bool,
    /// The setup version finished (`desktop::setup_gaps::SETUP_VERSION`): a
    /// lower one brings back the setup steps a release added.
    pub setup_version: u32,
    /// Stops every task at once, and the speech: Control-Option-Escape by
    /// default. Never Command-Option-Escape, which is macOS's Force Quit.
    pub stop_all_hotkey: Option<HotkeyBinding>,
    /// Real input for the Here lane (`background_physical_input` is the
    /// other): a Here task may move the real pointer and press real keys in
    /// the window in front (under a "Hands off" pill); off keeps it
    /// headless too.
    pub here_physical_input: bool,
    /// Web work runs in agent-browser's own hidden browsers, one session per
    /// task, instead of in the person's browser windows.
    pub background_browser: bool,
    /// Every new task browser starts with the person's accounts (the
    /// sign-in state); a task may still ask for a clean one.
    pub browser_signed_in_by_default: bool,
    /// The Chrome profile (`Default`, `Profile 1`) whose logins Hex's
    /// browsers take over, refreshed daily; `None` takes none.
    pub browser_import_profile: Option<String>,
    /// Apps no task may read or operate (by name or bundle id).
    pub blocked_apps: Vec<String>,
    /// Full control of this Mac: tasks of either lane may open any app and
    /// operate every window, the one in front of the person too, as the
    /// ChatGPT app's agent does; off keeps the person's windows theirs and
    /// tasks work in windows of their own. Was `background_front_window`.
    #[serde(alias = "background_front_window")]
    pub full_control: bool,
    /// Where coding jobs create their projects (`~` is the home folder).
    pub projects_folder: String,
    /// A question or chat running longer than this many minutes is stopped;
    /// 0 never.
    pub task_timeout_minutes: u32,
    /// A Background job's time budget in minutes (up to seven days, its
    /// default); 0 never stops it. It survives restarts (`resume_jobs`).
    pub job_budget_minutes: u32,
    /// A task that hits its agent's usage limit waits and retries.
    pub retry_on_limit: bool,
    /// Windows Background tasks open are parked off-screen, out of the
    /// person's way, until brought forward from the task list. On by
    /// default. Was `agent_space`.
    #[serde(alias = "agent_space")]
    pub park_windows: bool,
    /// The display new Background windows go to, by agent-desktop id ("5")
    /// or name ("M27Q"); `None` is the one the person isn't working on
    /// (with two or more displays; with one, windows stay where they open).
    pub agent_display: Option<String>,
    /// The desktop driver tasks act through (`assistant::driver`): Cua
    /// Driver, or agent-desktop; the other stands in when it is missing.
    pub desktop_driver: crate::assistant::driver::Driver,
    /// How freely tasks take consequential actions (send, delete, pay,
    /// share, publish): ask each time, only what the request named, or yolo.
    pub autonomy: crate::assistant::autonomy::Autonomy,
    /// A short note about the person (business, people, tone) every task
    /// reads; edited in Settings and by `hex_remember`. Stored beside the
    /// conversations, not here; this only turns it on.
    pub memory: bool,
    /// The shorter of the two follow-up windows (`follow_up_seconds` is the
    /// other): after a spoken answer, Hex keeps listening this many seconds
    /// for a follow-up without the shortcut; 0 turns conversation mode off.
    pub conversation_seconds: u32,
    /// Speaking while an answer is read aloud interrupts it (needs
    /// headphones or Hex's echo suppression).
    pub barge_in: bool,
    /// macOS notifications when a job finishes, fails, or asks something.
    pub notifications: bool,
    /// The computer doesn't go to sleep while a Background job works
    /// (`desktop::keep_awake`); the display still may.
    pub keep_awake: bool,
    /// With `keep_awake`, the display stays on too.
    pub keep_display_awake: bool,
    /// While the person is away, questions, approvals, failures and results
    /// go to their iPhone by iMessage, and their replies come back
    /// (`assistant::phone`).
    pub phone_enabled: bool,
    /// The person's own phone number or Apple ID that iMessages go to and
    /// are read from; nothing else is.
    pub phone_handle: Option<String>,
    /// Away after this many minutes without input (or the screen locked, or
    /// „ich gehe jetzt …“); 0 only by saying so or the manual switch.
    pub phone_away_minutes: u32,
}

/// The longest a job may run: seven days.
pub const JOB_BUDGET_MAX_MINUTES: u32 = 7 * 24 * 60;

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
            // Measured faster to its first spoken sentence than Haiku 5.5,
            // which thinks before it writes (docs/features/assistant.md).
            here_model: Some("claude-sonnet-5-5".into()),
            here_effort: Some("low".into()),
            escalate_after_tools: 8,
            warm_minutes: 15,
            speak_answers: true,
            voice: None,
            control_desktop: true,
            background_physical_input: false,
            include_screenshot: false,
            follow_up_seconds: 120,
            max_parallel: crate::assistant::capacity::recommended_parallel(),
            setup_done: false,
            setup_version: crate::desktop::setup_gaps::SETUP_VERSION,
            stop_all_hotkey: Some(HotkeyBinding::stop_all_default()),
            here_physical_input: true,
            background_browser: true,
            browser_signed_in_by_default: true,
            browser_import_profile: None,
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
            full_control: true,
            projects_folder: "~/dev".into(),
            task_timeout_minutes: 30,
            job_budget_minutes: JOB_BUDGET_MAX_MINUTES,
            retry_on_limit: true,
            park_windows: true,
            agent_display: None,
            desktop_driver: crate::assistant::driver::Driver::Cua,
            autonomy: crate::assistant::autonomy::Autonomy::Smart,
            memory: true,
            conversation_seconds: 8,
            barge_in: true,
            notifications: true,
            keep_awake: true,
            keep_display_awake: false,
            phone_enabled: false,
            phone_handle: None,
            phone_away_minutes: 10,
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
    // Hex's own voice loads when it's chosen, not with the first answer.
    let neural = crate::assistant::enabled(settings) && settings.assistant.speak_answers;
    crate::assistant::speech::neural::keep_warm(
        settings
            .assistant
            .voice
            .as_deref()
            .filter(|_| neural)
            .and_then(crate::assistant::speech::neural::preset),
    );
    *ASSISTANT_RUNTIME
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = Some((
        crate::assistant::enabled(settings),
        settings.assistant.clone(),
    ));
}

static ASSISTANT_RUNTIME: OnceLock<RwLock<Option<(bool, AssistantSettings)>>> = OnceLock::new();
