//! The model rows as dropdowns: the Claude models Agent-Desk knows (Opus, Sonnet,
//! Haiku, Fable, newest first), "Default", and "Other…", which shows the
//! field for any other model id. Codex and OpenCode name their models
//! themselves, so they keep the field.

use rune::{El, row};

use super::super::super::Menu;
use super::super::super::controls::Paint;
use super::super::super::general::quiet_select;
use super::super::Ui;
use crate::app_settings::{AssistantBackend, AssistantSettings};
use crate::desktop::i18n::tr;

/// The Claude models offered, by id, with the name the dropdown shows.
pub(in crate::rune_ui) const CLAUDE_MODELS: [(&str, &str); 4] = [
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
    ("claude-fable-5-2", "Claude Fable 5.2"),
];

/// Where a saved model sits in the dropdown: Default, one of
/// [`CLAUDE_MODELS`], or Other (any other id).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Choice {
    Default,
    Known(usize),
    Other,
}

/// The dropdown's choice for `saved`, where `default` is what an empty
/// setting means for this backend (the Claude API saves its default id).
pub(in crate::rune_ui) fn choice(saved: Option<&str>, default: Option<&str>) -> Choice {
    let Some(saved) = saved.map(str::trim).filter(|saved| !saved.is_empty()) else {
        return Choice::Default;
    };
    if default == Some(saved) {
        return Choice::Default;
    }
    let short = |id: &str| {
        id.trim_start_matches("claude-")
            .split('-')
            .next()
            .map(str::to_owned)
    };
    CLAUDE_MODELS
        .iter()
        .position(|(id, _)| *id == saved || short(id).as_deref() == Some(saved))
        .map_or(Choice::Other, Choice::Known)
}

/// Whether `backend` runs Claude models Agent-Desk can list.
pub(in crate::rune_ui) fn lists_models(backend: AssistantBackend) -> bool {
    matches!(
        backend,
        AssistantBackend::ClaudeCode | AssistantBackend::ClaudeApi
    )
}

/// The backend's saved model, and what Default stands for.
fn saved(settings: &AssistantSettings) -> (Option<String>, Option<&'static str>) {
    match settings.backend {
        AssistantBackend::ClaudeApi => (
            Some(settings.claude_api_model.clone()),
            Some("claude-sonnet-5-5"),
        ),
        _ => (settings.claude_code_model.clone(), None),
    }
}

fn store(settings: &mut AssistantSettings, model: Option<String>) {
    match settings.backend {
        AssistantBackend::ClaudeApi => {
            settings.claude_api_model =
                model.unwrap_or_else(|| AssistantSettings::default().claude_api_model);
        }
        _ => settings.claude_code_model = model,
    }
}

fn labels() -> Vec<String> {
    let mut labels = vec![tr("Default").to_owned()];
    labels.extend(CLAUDE_MODELS.iter().map(|(_, name)| (*name).to_owned()));
    labels.push(tr("Other…").to_owned());
    labels
}

fn index(choice: Choice) -> usize {
    match choice {
        Choice::Default => 0,
        Choice::Known(at) => at + 1,
        Choice::Other => CLAUDE_MODELS.len() + 1,
    }
}

/// The agent's model: a dropdown, with the field beside it for Other.
pub(in crate::rune_ui) fn model_select(ui: &Ui, p: Paint, layer: &mut Vec<El>, field: El) -> El {
    let (model, default) = saved(&ui.settings.assistant);
    let current = if ui.chats.settings.custom_model {
        Choice::Other
    } else {
        choice(model.as_deref(), default)
    };
    let select = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-model"),
        "assistant-model-control".to_owned(),
        tr("Model"),
        labels(),
        index(current),
        |ui, chosen| {
            ui.chats.settings.custom_model = chosen == CLAUDE_MODELS.len() + 1;
            if ui.chats.settings.custom_model {
                return;
            }
            let model = chosen
                .checked_sub(1)
                .and_then(|at| CLAUDE_MODELS.get(at))
                .map(|(id, _)| (*id).to_owned());
            super::set_model_text(ui, model.clone().unwrap_or_default());
            ui.update(|settings| store(&mut settings.assistant, model));
        },
        layer,
    );
    let mut controls = row().key("assistant-model-controls").gap(8.0).center_y();
    if current == Choice::Other {
        controls = controls.child(field);
    }
    controls.child(select)
}

/// The model quick questions about the window in front use: Default (the
/// agent's own) or one of [`CLAUDE_MODELS`].
pub(in crate::rune_ui) fn here_select(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let saved = ui.settings.assistant.here_model.as_deref();
    let current = match choice(saved, None) {
        Choice::Known(at) => at + 1,
        _ => 0,
    };
    let mut labels = vec![tr("Same as the agent").to_owned()];
    labels.extend(CLAUDE_MODELS.iter().map(|(_, name)| (*name).to_owned()));
    quiet_select(
        ui,
        p,
        Menu::Pick("assistant-here-model"),
        "assistant-here-model-control".to_owned(),
        tr("Quick questions use"),
        labels,
        current,
        |ui, chosen| {
            let model = chosen
                .checked_sub(1)
                .and_then(|at| CLAUDE_MODELS.get(at))
                .map(|(id, _)| (*id).to_owned());
            ui.update(|settings| settings.assistant.here_model = model);
        },
        layer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_models_find_their_place_in_the_dropdown() {
        assert_eq!(choice(None, None), Choice::Default);
        assert_eq!(choice(Some("  "), None), Choice::Default);
        assert_eq!(choice(Some("claude-opus-5-5"), None), Choice::Known(0));
        assert_eq!(choice(Some("opus"), None), Choice::Known(0), "an alias");
        assert_eq!(choice(Some("claude-haiku-5-5"), None), Choice::Known(2));
        assert_eq!(choice(Some("my-finetune"), None), Choice::Other);
        // The Claude API saves its default id: that reads as Default.
        assert_eq!(
            choice(Some("claude-sonnet-5-5"), Some("claude-sonnet-5-5")),
            Choice::Default
        );
        assert_eq!(index(Choice::Other), labels().len() - 1);
    }
}
