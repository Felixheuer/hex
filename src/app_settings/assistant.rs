//! Labs › Assistant's settings (macOS): the Action shortcut, the model that
//! answers, and how far the assistant may act. The Lab's switch is
//! `labs.assistant`; the Anthropic API key lives in the Keychain, never here.

use super::*;

/// Which model answers the Action shortcut and Chats.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantProvider {
    /// Anthropic's Messages API with its computer-use tool.
    #[default]
    Claude,
    /// The OpenCode service already running on this computer.
    OpenCode,
}

/// Labs › Assistant's options.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AssistantSettings {
    /// Hold to ask the assistant: right Command by default.
    pub action_hotkey: Option<HotkeyBinding>,
    pub provider: AssistantProvider,
    /// The Claude model id, such as `claude-sonnet-5-5`.
    pub claude_model: String,
    /// OpenCode's `provider/model`; `None` is OpenCode's own default.
    pub opencode_model: Option<String>,
    /// Answers to the Action shortcut are read aloud.
    pub speak_answers: bool,
    /// The macOS voice name; `None` is the system voice.
    pub voice: Option<String>,
    /// The assistant may click, type, and open apps without asking.
    pub control_desktop: bool,
    /// The Action shortcut sends a screenshot of the screen with the question.
    pub include_screenshot: bool,
    /// An Action press within this many seconds of the last answer continues
    /// that conversation; 0 always starts a new one.
    pub follow_up_seconds: u32,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            action_hotkey: Some(HotkeyBinding::action_default()),
            provider: AssistantProvider::Claude,
            claude_model: "claude-sonnet-5-5".into(),
            opencode_model: None,
            speak_answers: true,
            voice: None,
            control_desktop: true,
            include_screenshot: true,
            follow_up_seconds: 30,
        }
    }
}

impl HotkeyBinding {
    /// The Action shortcut's default: hold right Command alone.
    pub fn action_default() -> Self {
        Self {
            modifiers: HotkeyModifiers {
                command: Some(ModifierSide::Right),
                ..Default::default()
            },
            key: None,
        }
    }
}
