//! Chats › Settings: the agent that answers and its model, the Claude API
//! key, reading answers aloud, how far tasks may act on the desktop (real
//! input in the window in front, the hidden browser, blocked apps), the
//! follow-up window, how many tasks run at once and for how long, and Stop
//! all's shortcut (`lanes`).

mod lanes;
pub(super) mod more;

use std::time::Instant;

use rune::{El, TextEdit};
use rune_kit::TextInput;

use super::super::Menu;
use super::super::controls::{CardRow, Paint, described_row, section, setting_row, toggle_row};
use super::super::general::quiet_select;
use super::{BACKENDS, Job, Ui};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::backends::Found;
use crate::assistant::browser::AgentBrowser;
use crate::desktop::i18n::{tr, tr_fill};

/// Follow-up windows offered, in seconds; 0 always starts a new conversation.
const FOLLOW_UP: [u32; 6] = [0, 10, 20, 30, 60, 120];
/// Parallel task limits offered.
const PARALLEL: [u32; 12] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64];
/// How long typing in a model field pauses before it's saved.
const SAVE_AFTER: f64 = super::MODEL_SAVE_DELAY.as_secs_f64();

/// Settings' lists and text fields.
#[derive(Default)]
pub(in crate::rune_ui) struct SettingsState {
    /// The agents found on this Mac, once looked for.
    pub(in crate::rune_ui) found: Option<Vec<Found>>,
    /// The installed voices, once listed.
    pub(in crate::rune_ui) voices: Option<Vec<String>>,
    /// Each backend's model, in [`BACKENDS`] order, as typed.
    pub(in crate::rune_ui) models: [TextEdit; 4],
    /// When a model field last changed without being saved.
    model_typed: Option<Instant>,
    /// The Claude API key as typed; cleared once saved.
    pub(in crate::rune_ui) key: TextEdit,
    pub(in crate::rune_ui) key_error: Option<String>,
    /// The apps running now, once listed, to block one.
    pub(in crate::rune_ui) running: Option<Vec<String>>,
    /// An app to block, as typed.
    pub(in crate::rune_ui) blocked: TextEdit,
    /// agent-browser, once read.
    pub(in crate::rune_ui) browser: Option<AgentBrowser>,
    /// The connected displays, read when Settings or the onboarding opens.
    pub(in crate::rune_ui) displays: Vec<super::display_shim::Display>,
    /// What Hex knows about you, as edited, and whether that's saved.
    pub(in crate::rune_ui) memory: TextEdit,
    pub(in crate::rune_ui) memory_saved: bool,
    /// The agent picker's setups and key panels.
    pub(in crate::rune_ui) agents: super::agents::Agents,
    /// The engine clean browsers run, once read.
    pub(in crate::rune_ui) engine: Option<crate::assistant::browser::Engine>,
    /// Lightpanda installs from Settings now.
    pub(in crate::rune_ui) installing_lightpanda: bool,
    /// Someone signed in to sites in Hex's own browser.
    pub(in crate::rune_ui) signed_in: bool,
    /// Hex's browser shows for signing in.
    pub(in crate::rune_ui) signing_in: bool,
    pub(in crate::rune_ui) sign_in_error: Option<String>,
}

fn index(backend: AssistantBackend) -> usize {
    BACKENDS
        .iter()
        .position(|known| *known == backend)
        .unwrap_or(0)
}

/// A backend's saved model, as its field shows it.
fn saved_model(settings: &AssistantSettings, backend: AssistantBackend) -> String {
    match backend {
        AssistantBackend::ClaudeCode => settings.claude_code_model.clone().unwrap_or_default(),
        AssistantBackend::Codex => settings.codex_model.clone().unwrap_or_default(),
        AssistantBackend::OpenCode => settings.opencode_model.clone().unwrap_or_default(),
        AssistantBackend::ClaudeApi => settings.claude_api_model.clone(),
    }
}

/// `settings` with every model field's text.
fn with_models(settings: &AssistantSettings, models: &[TextEdit; 4]) -> AssistantSettings {
    let mut next = settings.clone();
    let typed = |backend: AssistantBackend| {
        let text = models[index(backend)].text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    };
    next.claude_code_model = typed(AssistantBackend::ClaudeCode);
    next.codex_model = typed(AssistantBackend::Codex);
    next.opencode_model = typed(AssistantBackend::OpenCode);
    next.claude_api_model = typed(AssistantBackend::ClaudeApi)
        .unwrap_or_else(|| AssistantSettings::default().claude_api_model);
    next
}

/// Settings opened: its fields show what's saved, and the agents and voices
/// are looked up.
pub(super) fn opened(ui: &mut Ui) {
    for backend in BACKENDS {
        ui.chats.settings.models[index(backend)] =
            TextEdit::new(saved_model(&ui.settings.assistant, backend));
    }
    ui.chats.settings.model_typed = None;
    super::displays::read(ui);
    more::opened(ui);
    ui.chats.jobs.run(Job::Detect);
    ui.chats.jobs.run(Job::Browser);
    ui.chats.jobs.run(Job::RunningApps);
    if ui.chats.settings.voices.is_none() {
        ui.chats.jobs.run(Job::Voices);
    }
}

/// Saves model fields once typing pauses.
pub(super) fn poll(ui: &mut Ui, now: Instant) -> bool {
    let Some(typed) = ui.chats.settings.model_typed else {
        return false;
    };
    if now.duration_since(typed).as_secs_f64() < SAVE_AFTER {
        return false;
    }
    flush(ui);
    true
}

/// Saves model fields now (leaving Settings).
pub(super) fn flush(ui: &mut Ui) {
    if ui.chats.settings.model_typed.take().is_none() {
        return;
    }
    let next = with_models(&ui.settings.assistant, &ui.chats.settings.models);
    if next != ui.settings.assistant {
        ui.update(|settings| settings.assistant = next);
    }
}

/// Saves the typed Claude API key in the Keychain.
pub(super) fn save_key(ui: &mut Ui) {
    let key = ui.chats.settings.key.text.trim().to_owned();
    ui.chats.settings.key_error = None;
    ui.chats.jobs.run(Job::SaveKey(key));
}

pub(super) fn page_rows(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<El> {
    let assistant = &ui.settings.assistant;
    let agent: Vec<CardRow> = vec![
        // The onboarding's agent picker: switch agents or keys here.
        super::agent_picker::picker(ui, p).px(4.0).into(),
        setting_row(
            p,
            "assistant-model",
            tr("Model"),
            Some(tr("Empty uses the agent's own default")),
            model_field(ui, p),
        )
        .into(),
    ];
    let mut voice: Vec<CardRow> = vec![
        toggle_row(
            p,
            "assistant-speak",
            tr("Read answers aloud"),
            tr("Quick questions answer out loud as well as in the HUD"),
            assistant.speak_answers,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.speak_answers ^= true);
            },
        )
        .into(),
    ];
    if assistant.speak_answers {
        voice.push(more::voice_row(ui, p, layer).into());
        voice.extend(more::better_voice_row(ui, p));
    }
    voice.extend(more::conversation_rows(ui, p, layer));
    voice.push(more::notifications_row(ui, p));
    let physical = if assistant.physical_input {
        Some((
            tr("Tasks move your pointer and type with your keyboard. Don't use them while a task runs").to_owned(),
            p.warning(),
        ))
    } else {
        Some((
            tr("Off keeps every action in the background, without your pointer or keys").to_owned(),
            p.muted(),
        ))
    };
    let mut desktop: Vec<CardRow> = vec![
        toggle_row(
            p,
            "assistant-control-desktop",
            tr("Operate apps"),
            tr("Tasks click, type, and read apps through agent-desktop"),
            assistant.control_desktop,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.control_desktop ^= true);
            },
        )
        .into(),
    ];
    if assistant.control_desktop {
        desktop.push(lanes::here_input_row(ui, p));
        desktop.push(
            super::super::controls::pressable_row(
                described_row(
                    p,
                    "assistant-physical-input",
                    tr("Use the real pointer and keyboard"),
                    physical,
                    super::super::controls::row_switch(p, assistant.physical_input),
                ),
                assistant.physical_input,
            )
            .on_press(|ui: &mut Ui| {
                ui.update(|settings| settings.assistant.physical_input ^= true);
            })
            .into(),
        );
    }
    desktop.push(super::displays::settings_row(ui, p, layer).into());
    desktop.push(
        toggle_row(
            p,
            "assistant-screenshot",
            tr("Send a screenshot"),
            tr("A quick question includes the screen the pointer is on"),
            assistant.include_screenshot,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.include_screenshot ^= true);
            },
        )
        .into(),
    );
    let mut tasks = vec![
        lanes::stop_all_row(ui, p),
        setting_row(
            p,
            "assistant-follow-up",
            tr("Follow-up window"),
            Some(tr(
                "A question this soon after an answer continues that chat",
            )),
            follow_up_select(ui, p, layer),
        ),
        setting_row(
            p,
            "assistant-parallel",
            tr("Tasks at once"),
            Some(tr("More wait their turn")),
            parallel_select(ui, p, layer),
        ),
    ];
    tasks.extend(lanes::task_rows(ui, p, layer));
    vec![
        section(p, "assistant-agent", tr("Agent"), agent),
        more::autonomy_section(ui, p),
        section(p, "assistant-voice-section", tr("Answers"), voice),
        section(p, "assistant-tasks", tr("Tasks"), tasks),
        more::memory_section(ui, p),
        section(p, "assistant-desktop", tr("Desktop"), desktop),
        lanes::web_section(ui, p),
        lanes::blocked_section(ui, p, layer),
        lanes::experimental_section(ui, p),
    ]
}

/// The settings page.
pub(super) fn page(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    super::super::controls::page("chats-settings")
        .pb(24.0)
        .children(page_rows(ui, p, layer))
}

fn model_field(ui: &Ui, p: Paint) -> El {
    let backend = ui.settings.assistant.backend;
    let slot = index(backend);
    let placeholder = match backend {
        AssistantBackend::OpenCode => "provider/model",
        AssistantBackend::ClaudeApi => "claude-sonnet-5-5",
        _ => tr("Default"),
    };
    TextInput::new(&ui.chats.settings.models[slot])
        .placeholder(placeholder)
        .label(tr("Model"))
        .width(Some(220.0))
        .on_edit(move |ui: &mut Ui| &mut ui.chats.settings.models[slot])
        .on_change(|ui: &mut Ui, _| ui.chats.settings.model_typed = Some(Instant::now()))
        .build(p.k)
        .key("assistant-model-field")
}

fn seconds_label(seconds: u32) -> String {
    match seconds {
        0 => tr("Off").to_owned(),
        60.. if seconds.is_multiple_of(60) => tr_fill("{} min", &(seconds / 60).to_string()),
        _ => tr_fill("{} s", &seconds.to_string()),
    }
}

/// `values` with `saved` among them, sorted, and where `saved` is.
fn with_saved(values: &[u32], saved: u32) -> (Vec<u32>, usize) {
    let mut values = values.to_vec();
    if !values.contains(&saved) {
        values.push(saved);
        values.sort_unstable();
    }
    let index = values.iter().position(|value| *value == saved).unwrap_or(0);
    (values, index)
}

fn follow_up_select(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let (values, selected) = with_saved(&FOLLOW_UP, ui.settings.assistant.follow_up_seconds);
    quiet_select(
        ui,
        p,
        Menu::Pick("assistant-follow-up"),
        "assistant-follow-up-control".to_owned(),
        tr("Follow-up window"),
        values
            .iter()
            .map(|seconds| seconds_label(*seconds))
            .collect(),
        selected,
        move |ui, chosen| {
            let seconds = values[chosen];
            ui.update(|settings| settings.assistant.follow_up_seconds = seconds);
        },
        layer,
    )
}

fn parallel_select(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let saved = ui.settings.assistant.max_parallel.clamp(1, 64);
    let (values, selected) = with_saved(&PARALLEL, saved);
    quiet_select(
        ui,
        p,
        Menu::Pick("assistant-parallel"),
        "assistant-parallel-control".to_owned(),
        tr("Tasks at once"),
        values.iter().map(u32::to_string).collect(),
        selected,
        move |ui, chosen| {
            let limit = values[chosen];
            ui.update(|settings| settings.assistant.max_parallel = limit);
        },
        layer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_fields_save_trimmed_and_empty_is_the_default() {
        let saved = AssistantSettings::default();
        let mut models: [TextEdit; 4] = Default::default();
        models[index(AssistantBackend::Codex)] = TextEdit::new("  gpt-6 ");
        let next = with_models(&saved, &models);
        assert_eq!(next.codex_model.as_deref(), Some("gpt-6"));
        assert_eq!(next.claude_code_model, None);
        assert_eq!(next.claude_api_model, saved.claude_api_model);
    }

    #[test]
    fn a_saved_value_outside_the_choices_stays_choosable() {
        assert_eq!(with_saved(&PARALLEL, 8), (PARALLEL.to_vec(), 5));
        let (values, index) = with_saved(&PARALLEL, 5);
        assert_eq!(values[index], 5);
        assert_eq!(values.len(), PARALLEL.len() + 1);
    }
}
