//! The former Labs features that are Dictation settings now: what happens
//! to the text, and the Advanced choices (the mic's linger, unloading the
//! speech model, Caps Lock).

use rune::El;

use super::super::controls::{CardRow, Paint, icon_row, icon_toggle_row};
use super::super::icons::*;
use super::super::{Menu, Ui};
use super::{pick_row, quiet_select, search};
use crate::desktop::i18n::{tr, tr_fill};

/// What happens to a dictation's text: paste (and the clipboard), live
/// text, spoken formatting, and filler words. Former Labs, now settings.
pub(in crate::rune_ui) fn text_rows(
    ui: &Ui,
    p: Paint,
    layer: &mut Vec<El>,
    query: &str,
) -> Vec<CardRow> {
    use crate::app_settings::AfterDictation;
    let preferences = &ui.settings.preferences;
    let mut rows = search::Rows::new(query);
    let choices: Vec<AfterDictation> = AfterDictation::ALL
        .into_iter()
        .filter(|choice| {
            *choice != AfterDictation::CopyOnly
                || ui.platform.caps.os != super::super::platform::Os::Linux
        })
        .collect();
    rows.push(
        tr("After dictating"),
        pick_row(
            ui,
            p,
            "after-dictation",
            &PASTE,
            tr("After dictating"),
            choices.iter().map(|choice| tr(choice.name())).collect(),
            choices
                .iter()
                .position(|choice| *choice == preferences.after_dictation)
                .unwrap_or(0),
            |ui, index| {
                let choice = AfterDictation::ALL[index];
                ui.update(|settings| settings.preferences.after_dictation = choice);
            },
            layer,
        ),
    );
    rows.push_described(
        tr("Live text"),
        Some(tr("Shows the words beside the HUD while you speak")),
        icon_toggle_row(
            p,
            "live-text",
            &PANEL,
            tr("Live text"),
            tr("Shows the words beside the HUD while you speak"),
            preferences.live_text,
            |ui: &mut Ui| {
                ui.update(|settings| settings.preferences.live_text ^= true);
            },
        ),
    );
    if ui.platform.caps.os == super::super::platform::Os::Linux {
        rows.push_described(
            tr("Live typing"),
            Some(tr(
                "Types the words into the focused field while you speak (Wayland)",
            )),
            icon_toggle_row(
                p,
                "live-typing",
                &KEYBOARD,
                tr("Live typing"),
                tr("Types the words into the focused field while you speak (Wayland)"),
                preferences.live_typing,
                |ui: &mut Ui| {
                    ui.update(|settings| settings.preferences.live_typing ^= true);
                },
            ),
        );
    }
    rows.push_described(
        tr("Spoken formatting"),
        Some(tr(
            "Say \"new line\", \"comma\", or \"open quote\" for the character",
        )),
        icon_toggle_row(
            p,
            "spoken-formatting",
            &PEN,
            tr("Spoken formatting"),
            tr("Say \"new line\", \"comma\", or \"open quote\" for the character"),
            preferences.spoken_formatting,
            |ui: &mut Ui| {
                ui.update(|settings| settings.preferences.spoken_formatting ^= true);
            },
        ),
    );
    rows.push_described(
        tr("Remove filler words"),
        Some(tr("Drops \"um\" and \"uh\" from what you say")),
        icon_toggle_row(
            p,
            "filler-words",
            &ZAP,
            tr("Remove filler words"),
            tr("Drops \"um\" and \"uh\" from what you say"),
            preferences.remove_filler_words,
            |ui: &mut Ui| {
                ui.update(|settings| settings.preferences.remove_filler_words ^= true);
            },
        ),
    );
    rows.into_rows()
}

/// Off, or how long the mic stays open after a dictation.
pub(super) fn keep_microphone_open_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> CardRow {
    use crate::app_settings::KEEP_MICROPHONE_OPEN_SECONDS;
    let mut labels = vec![tr("Off").to_owned()];
    labels.extend(KEEP_MICROPHONE_OPEN_SECONDS.iter().map(|seconds| {
        if *seconds < 60 {
            tr_fill("{} s", &seconds.to_string())
        } else {
            tr_fill("{} min", &(seconds / 60).to_string())
        }
    }));
    let selected = ui
        .settings
        .preferences
        .keep_microphone_open_seconds
        .and_then(|saved| {
            KEEP_MICROPHONE_OPEN_SECONDS
                .iter()
                .position(|s| *s == saved)
        })
        .map_or(0, |index| index + 1);
    let control = quiet_select(
        ui,
        p,
        Menu::Pick("keep-microphone-open"),
        "keep-microphone-open-control".to_owned(),
        tr("Keep the mic open after dictating"),
        labels,
        selected,
        |ui, index| {
            let seconds = index
                .checked_sub(1)
                .map(|index| KEEP_MICROPHONE_OPEN_SECONDS[index]);
            ui.update(|settings| settings.preferences.keep_microphone_open_seconds = seconds);
        },
        layer,
    );
    icon_row(
        p,
        "keep-microphone-open",
        &MIC,
        tr("Keep the mic open after dictating"),
        Some(tr("The next dictation starts at once")),
        control,
    )
    .into()
}

/// Never, or after how many idle minutes the speech model unloads.
pub(super) fn unload_model_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> CardRow {
    use crate::app_settings::UNLOAD_IDLE_MINUTES;
    let mut labels = vec![tr("Never").to_owned()];
    labels.extend(
        UNLOAD_IDLE_MINUTES
            .iter()
            .map(|minutes| tr_fill("After {} min", &minutes.to_string())),
    );
    let selected = ui
        .settings
        .preferences
        .unload_model_after_minutes
        .and_then(|saved| UNLOAD_IDLE_MINUTES.iter().position(|m| *m == saved))
        .map_or(0, |index| index + 1);
    let control = quiet_select(
        ui,
        p,
        Menu::Pick("unload-model"),
        "unload-model-control".to_owned(),
        tr("Unload the speech model"),
        labels,
        selected,
        |ui, index| {
            let minutes = index.checked_sub(1).map(|index| UNLOAD_IDLE_MINUTES[index]);
            ui.update(|settings| settings.preferences.unload_model_after_minutes = minutes);
        },
        layer,
    );
    icon_row(
        p,
        "unload-model",
        &TIMER,
        tr("Unload the speech model"),
        Some(tr("Frees its memory while you don't dictate")),
        control,
    )
    .into()
}

/// Caps Lock as a dictation key, and its light (macOS).
pub(super) fn caps_lock_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> CardRow {
    use crate::app_settings::CapsLockMode;
    pick_row(
        ui,
        p,
        "caps-lock",
        &KEYBOARD,
        tr("Caps Lock"),
        CapsLockMode::ALL
            .iter()
            .map(|mode| tr(mode.name()))
            .collect(),
        CapsLockMode::ALL
            .iter()
            .position(|mode| *mode == ui.settings.preferences.caps_lock)
            .unwrap_or(0),
        |ui, index| {
            let mode = CapsLockMode::ALL[index];
            ui.update(|settings| settings.preferences.caps_lock = mode);
        },
        layer,
    )
}
