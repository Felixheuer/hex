//! The assistant (macOS): what else takes the dictation or Action
//! shortcut's keys (`shortcut_conflicts`), as notes under its row, each with
//! where it is fixed and "Choose another shortcut". No conflict, or the
//! assistant off, adds nothing.

use super::*;
use crate::app_settings::HotkeySlot;
use crate::shortcut_conflicts::{Certainty, Conflict, Fix, Source};

/// The notes under `target`'s row.
pub(super) fn rows(ui: &Ui, p: Paint, key: &str, target: ShortcutTarget) -> Vec<CardRow> {
    if !crate::assistant::enabled(&ui.settings) {
        return Vec::new();
    }
    let slot = match target {
        ShortcutTarget::Dictation => HotkeySlot::Dictation,
        ShortcutTarget::Action => HotkeySlot::Action,
        ShortcutTarget::StopAll => HotkeySlot::StopAll,
        _ => return Vec::new(),
    };
    crate::shortcut_conflicts::cached(slot, &ui.settings)
        .into_iter()
        .enumerate()
        .map(|(index, conflict)| {
            conflict_row(p, &format!("{key}-conflict-{index}"), target, conflict).into()
        })
        .collect()
}

fn conflict_row(p: Paint, key: &str, target: ShortcutTarget, conflict: Conflict) -> El {
    let copy = match (&conflict.source, conflict.certainty) {
        (Source::MacOs, Certainty::Taken) => tr_fill(
            "macOS uses it for “{}”, so Agent-Desk may never see it",
            &conflict.name,
        ),
        (Source::App(_), Certainty::Taken) => tr_fill(
            "{} uses it too, so Agent-Desk may never see it",
            &conflict.name,
        ),
        (_, Certainty::Possible) => tr_fill("{} may use it too", &conflict.name),
    };
    let color = match conflict.certainty {
        Certainty::Taken => p.danger(),
        Certainty::Possible => p.muted(),
    };
    let fix_label = match &conflict.fix {
        Fix::KeyboardSettings => tr("Open Keyboard Settings").to_owned(),
        Fix::SiriSettings => tr("Open Siri Settings").to_owned(),
        Fix::OpenApp { .. } => tr_fill("Open {}", &conflict.name),
    };
    let fix = conflict.fix.clone();
    col()
        .key(key)
        .px(METRICS.row_px)
        .py(8.0)
        .gap(8.0)
        .child(
            text(copy)
                .key("copy")
                .font(controls::DESCRIPTION)
                .color(color)
                .wrap(),
        )
        .child(
            row()
                .key("actions")
                .gap(8.0)
                .child(button(
                    p,
                    "fix",
                    &fix_label,
                    ButtonKind::Secondary,
                    move |_, _| fix.open(),
                ))
                .child(button(
                    p,
                    "choose-another",
                    tr("Choose another shortcut"),
                    ButtonKind::Secondary,
                    move |ui: &mut Ui, _| s::begin_capture(ui, target),
                )),
        )
}
