//! The Assistant pane (the main window's sidebar): the agent that answers
//! and its model, how freely it acts and how long and how many jobs run,
//! apps and windows (real input, the window in front, blocked apps,
//! displays), the web, its voice (one picker: the system voices and Agent-Desk's
//! own), notifications, and what it remembers, after its two shortcuts
//! (Action, and Stop all tasks).

mod lanes;
mod models;
pub(super) mod more;
mod phone;

use std::time::Instant;

use rune::{El, TextEdit};
use rune_kit::TextInput;

use super::super::Menu;
use super::super::controls::{CardRow, Paint, section, setting_row, toggle_row};
use super::super::general::quiet_select;
use super::{BACKENDS, Job, Ui};
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::assistant::backends::Found;
use crate::assistant::browser::AgentBrowser;
use crate::desktop::i18n::{tr, tr_fill};

/// "Continue" follow-up windows offered, in seconds; 0 always starts a new
/// conversation.
const FOLLOW_UP: [u32; 6] = [0, 10, 20, 30, 60, 120];
/// Parallel task limits offered (the saved one and this Mac's
/// recommendation join them).
const PARALLEL: [u32; 12] = [1, 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 50];
/// How long typing in a model field pauses before it's saved.
const SAVE_AFTER: f64 = super::MODEL_SAVE_DELAY.as_secs_f64();

/// Settings' lists and text fields.
#[derive(Default)]
pub(in crate::rune_ui) struct SettingsState {
    /// The agents found on this Mac, once looked for.
    pub(in crate::rune_ui) found: Option<Vec<Found>>,
    /// The installed voices, once listed.
    pub(in crate::rune_ui) voices: Option<Vec<String>>,
    /// Agent-Desk's voice downloading (its bytes so far) or why it couldn't.
    pub(in crate::rune_ui) voice_download: Option<Result<u64, String>>,
    /// Each backend's model, in [`BACKENDS`] order, as typed.
    pub(in crate::rune_ui) models: [TextEdit; 4],
    /// When a model field last changed without being saved.
    model_typed: Option<Instant>,
    /// "Other…" is chosen in the model dropdown: the field shows.
    pub(in crate::rune_ui) custom_model: bool,
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
    pub(in crate::rune_ui) displays: Vec<crate::assistant::computer::Display>,
    /// What Agent-Desk knows about you, as edited, and whether that's saved.
    pub(in crate::rune_ui) memory: TextEdit,
    pub(in crate::rune_ui) memory_saved: bool,
    /// Clear was pressed once: the next press empties what it knows.
    pub(in crate::rune_ui) memory_clear_armed: bool,
    /// The agent picker's setups and key panels.
    pub(in crate::rune_ui) agents: super::agents::Agents,
    /// The engine clean browsers run, once read.
    pub(in crate::rune_ui) engine: Option<crate::assistant::browser::Engine>,
    /// Lightpanda installs from Settings now.
    pub(in crate::rune_ui) installing_lightpanda: bool,
    /// Browser data is saved for at least one site; its login can expire.
    pub(in crate::rune_ui) signed_in: bool,
    /// Agent-Desk's browser shows for signing in.
    pub(in crate::rune_ui) signing_in: bool,
    pub(in crate::rune_ui) sign_in_busy: bool,
    pub(in crate::rune_ui) sign_in_editor: bool,
    pub(in crate::rune_ui) site_address: TextEdit,
    pub(in crate::rune_ui) saved_sites: Vec<String>,
    pub(in crate::rune_ui) sign_in_error: Option<String>,
    /// The iPhone channel's handle as typed, and what its rows show.
    pub(in crate::rune_ui) phone: phone::PhoneState,
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
    ui.chats.settings.custom_model = false;
    super::displays::read(ui);
    more::opened(ui);
    phone::opened(ui);
    ui.chats.jobs.run(Job::Detect);
    super::accounts::read(ui);
    ui.chats.jobs.run(Job::RunningApps);
    if ui.chats.settings.voices.is_none() {
        ui.chats.jobs.run(Job::Voices);
    }
}

/// The current backend's model field shows `text` (a model picked from
/// the dropdown), with nothing left to save.
fn set_model_text(ui: &mut Ui, text: String) {
    let slot = index(ui.settings.assistant.backend);
    ui.chats.settings.models[slot] = TextEdit::new(text);
    ui.chats.settings.model_typed = None;
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
        if models::lists_models(assistant.backend) {
            setting_row(
                p,
                "assistant-model",
                tr("Model"),
                Some(tr("Default uses the agent's own model")),
                models::model_select(ui, p, layer, model_field(ui, p)),
            )
            .into()
        } else {
            setting_row(
                p,
                "assistant-model",
                tr("Model"),
                Some(tr("Empty uses the agent's own default")),
                model_field(ui, p),
            )
            .into()
        },
    ];
    let mut agent = agent;
    if models::lists_models(assistant.backend) {
        agent.push(
            setting_row(
                p,
                "assistant-here-model",
                tr("Quick questions use"),
                Some(tr(
                    "A faster model answers questions about the window in front",
                )),
                models::here_select(ui, p, layer),
            )
            .into(),
        );
    }
    // How freely it acts, how long and how many jobs run, and keeping the
    // Mac awake for them.
    let mut works: Vec<CardRow> = vec![
        rune::row()
            .key("autonomy-row")
            .px(16.0)
            .py(12.0)
            .child(more::autonomy_choice(ui, p).grow(1.0))
            .into(),
    ];
    works.extend(
        lanes::task_rows(ui, p, layer)
            .into_iter()
            .map(CardRow::from),
    );
    works.push(
        setting_row(
            p,
            "assistant-parallel",
            tr("Tasks at once"),
            Some(parallel_note(assistant.max_parallel)),
            parallel_select(ui, p, layer),
        )
        .into(),
    );
    works.push(
        toggle_row(
            p,
            "assistant-keep-awake",
            tr("Keep the Mac awake while tasks run"),
            tr("Background tasks can run for hours or days. The display may sleep, but closing a laptop's lid on battery still sleeps it"),
            assistant.keep_awake,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.keep_awake ^= true);
            },
        )
        .into(),
    );
    if assistant.keep_awake {
        works.push(
            toggle_row(
                p,
                "assistant-keep-display-awake",
                tr("Keep the display awake too"),
                tr("The screen stays on while tasks run, so you can watch them"),
                assistant.keep_display_awake,
                |ui: &mut Ui| {
                    ui.update(|settings| settings.assistant.keep_display_awake ^= true);
                },
            )
            .into(),
        );
    }
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
        // Real input, one switch per lane, then full control.
        desktop.push(lanes::here_input_row(ui, p));
        desktop.push(lanes::background_input_row(ui, p));
        desktop.push(lanes::full_control_row(ui, p));
    }
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
    desktop.push(super::displays::settings_row(ui, p, layer).into());
    desktop.push(lanes::park_windows_row(ui, p).into());
    // One voice picker: the system voices and Agent-Desk's own, with its download.
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
        voice.extend(more::hex_voice_row(ui, p));
    }
    voice.push(more::follow_up_row(ui, p, layer));
    let mut sections = vec![super::switch::settings_card(ui, p)];
    if crate::assistant::enabled(&ui.settings) && ui.records_shortcuts() {
        sections.push(section(
            p,
            "assistant-shortcuts",
            tr("Shortcuts"),
            shortcut_rows(ui, p),
        ));
    }
    sections.extend([
        section(p, "assistant-agent", tr("Agent & model"), agent),
        section(p, "assistant-tasks", tr("How it works"), works),
        section(p, "assistant-desktop", tr("Apps & windows"), desktop),
        lanes::blocked_section(ui, p, layer),
        lanes::web_section(ui, p, layer),
        section(p, "assistant-voice-section", tr("Voice"), voice),
        section(
            p,
            "assistant-notifications",
            tr("Notifications"),
            vec![more::notifications_row(ui, p)],
        ),
        phone::section(ui, p, layer),
        more::memory_section(ui, p),
    ]);
    sections
}

/// The Action shortcut (hold and say anything, the assistant decides;
/// double-tap for the task list) and Stop all tasks under it.
fn shortcut_rows(ui: &Ui, p: Paint) -> Vec<CardRow> {
    use super::super::ShortcutTarget;
    use super::super::general::shortcut_rows as rows;
    use super::super::icons::{CIRCLE_X, KEYBOARD};
    let mut all = rows(
        ui,
        p,
        "action-shortcut",
        &KEYBOARD,
        tr("Action shortcut"),
        ShortcutTarget::Action,
        Some(tr("Hold and say anything; Agent-Desk decides")),
    );
    all.extend(rows(
        ui,
        p,
        "stop-all-shortcut",
        &CIRCLE_X,
        tr("Stop all tasks"),
        ShortcutTarget::StopAll,
        Some(tr("Stops every assistant task and its speech")),
    ));
    all
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

/// "More wait their turn", and a warning above this Mac's recommendation.
fn parallel_note(saved: u32) -> &'static str {
    if saved > crate::assistant::capacity::recommended_parallel() {
        tr("More wait their turn. Above the recommendation this Mac may get slow")
    } else {
        tr("More wait their turn")
    }
}

/// The parallel limits offered: the steps, the saved one, and this Mac's
/// recommendation, each label marking the recommendation.
fn parallel_choices(saved: u32, recommended: u32) -> (Vec<u32>, usize, Vec<String>) {
    let (mut values, _) = with_saved(&PARALLEL, saved);
    if !values.contains(&recommended) {
        values.push(recommended);
        values.sort_unstable();
    }
    let selected = values.iter().position(|value| *value == saved).unwrap_or(0);
    let labels = values
        .iter()
        .map(|value| {
            if *value == recommended {
                tr_fill("{} · Recommended", &value.to_string())
            } else {
                value.to_string()
            }
        })
        .collect();
    (values, selected, labels)
}

fn parallel_select(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let max = crate::assistant::capacity::MAX_PARALLEL;
    let saved = ui.settings.assistant.max_parallel.clamp(1, max);
    let recommended = crate::assistant::capacity::recommended_parallel();
    let (values, selected, labels) = parallel_choices(saved, recommended);
    quiet_select(
        ui,
        p,
        Menu::Pick("assistant-parallel"),
        "assistant-parallel-control".to_owned(),
        tr("Tasks at once"),
        labels,
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
        assert_eq!(with_saved(&PARALLEL, 8), (PARALLEL.to_vec(), 4));
        let (values, index) = with_saved(&PARALLEL, 5);
        assert_eq!(values[index], 5);
        assert_eq!(values.len(), PARALLEL.len() + 1);
    }

    #[test]
    fn the_recommendation_is_offered_and_marked() {
        let (values, selected, labels) = parallel_choices(32, 10);
        assert_eq!(values[selected], 32, "the saved limit stays chosen");
        let at = values.iter().position(|value| *value == 10).unwrap();
        assert!(labels[at].starts_with("10 ·"), "{}", labels[at]);
        assert_eq!(values.last(), Some(&50));
        assert_eq!(labels.iter().filter(|label| label.contains('·')).count(), 1);
    }
}
