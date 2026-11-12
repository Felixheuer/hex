//! The onboarding's Shortcut page: what else takes the tested shortcuts,
//! each with where it's fixed, then a live press of each.

use rune::{El, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, glyphs};

use super::super::super::controls::{Paint, PushSize};
use super::super::super::onboarding_kit::keys;
use super::super::super::platform::Os;
use super::super::Ui;
use super::super::setup::{self, Page, ProbeState, SetupFlow, page_done};
use super::super::shortcut::{ConflictFix, Probe};
use super::{actions, done_pill, lead, note, page, push, row_of, rows_card, spinner, waiting_pill};
use crate::app_settings::HotkeySlot;
use crate::desktop::i18n::{tr, tr_fill};

const ROW_H: f32 = super::ROW_H;

/// A conflict's fix button.
fn conflict_label(fix: &ConflictFix, name: &str) -> String {
    match fix {
        ConflictFix::KeyboardSettings => tr("Open Keyboard Settings").into(),
        ConflictFix::SiriSettings => tr("Open Siri Settings…").into(),
        ConflictFix::OpenApp(_) => tr_fill("Open {}", name),
    }
}

/// The shortcut's name, as General's rows call it.
fn slot_name(slot: HotkeySlot) -> &'static str {
    match slot {
        HotkeySlot::Action => tr("Action shortcut"),
        _ => tr("Dictation shortcut"),
    }
}

/// The keys of a tested shortcut.
fn slot_keys(ui: &Ui, p: Paint, slot: HotkeySlot) -> Option<El> {
    let binding = match slot {
        HotkeySlot::Action => ui.settings.assistant.action_hotkey.as_ref(),
        _ => Some(&ui.settings.dictation_hotkey),
    }?;
    let caps = super::super::super::settings::shortcut_caps(binding, false);
    Some(keys(p, "keys", &caps, false))
}

/// What a press test found, for a person; `taker` names who took the keys
/// when a conflict says so.
fn probe_words(found: &Probe, taker: Option<&str>, mac: bool) -> (String, bool) {
    match found {
        Probe::Works(held) => (
            tr_fill(
                "Hex received it (held {})",
                &format!("{:.1} s", held.as_secs_f32()),
            ),
            true,
        ),
        Probe::Swallowed => (
            match taker {
                Some(name) => tr_fill("{} took the keys before Hex could", name),
                None if mac => tr("Another app or macOS took the keys before Hex could").into(),
                None => tr("Another app or the system took the keys before Hex could").into(),
            },
            false,
        ),
        Probe::NothingReceived => (
            if mac {
                tr("Nothing reached Hex. Another app or macOS may take it")
            } else {
                tr("Nothing reached Hex. Another app or the system may take it")
            }
            .into(),
            false,
        ),
        Probe::WrongKeys(saw) => (tr_fill("Hex saw {} instead", saw), false),
        Probe::NoRelease => (tr("Hex never saw the keys let go").into(), false),
        Probe::Unavailable(why) => (why.clone(), false),
    }
}

/// One shortcut's live press test: Test, a waiting pill while it listens,
/// then what reached Hex.
fn probe_row(ui: &Ui, flow: &SetupFlow, p: Paint, slot: HotkeySlot) -> El {
    let taker = flow.conflicts.as_ref().and_then(|found| {
        found
            .iter()
            .find(|(tested, _)| *tested == slot)
            .map(|(_, conflict)| conflict.name.as_str())
    });
    let (detail, ink, control) = match setup::probe_state(flow, slot) {
        None => (
            tr("Press Test, then press and release it").to_owned(),
            p.muted(),
            push(
                p,
                "test",
                tr("Test"),
                slot == HotkeySlot::Action,
                move |ui: &mut Ui| setup::test_shortcut(ui, slot),
            ),
        ),
        Some(ProbeState::Waiting) => (
            tr("Press and release it now").to_owned(),
            p.muted(),
            waiting_pill(p, "waiting", tr("Listening")),
        ),
        Some(ProbeState::Done(found)) => {
            let mac = ui.platform.caps.os == Os::Macos;
            let (words, works) = probe_words(found, taker, mac);
            let control = if works {
                done_pill(p, "works", tr("Works"))
            } else {
                push(
                    p,
                    "test",
                    tr("Test again"),
                    slot == HotkeySlot::Action,
                    move |ui: &mut Ui| setup::test_shortcut(ui, slot),
                )
            };
            (words, if works { p.muted() } else { p.danger() }, control)
        }
    };
    let title = slot_name(slot);
    row()
        .key(format!("probe-{slot:?}").to_lowercase())
        .role("group")
        .label(title)
        .description(detail.clone())
        .min_h(ROW_H)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(
            icon(&glyphs::KEYBOARD, 16.0)
                .key("icon")
                .color(p.k.c.icon_base)
                .a11y_hidden(),
        )
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    row()
                        .key("title-row")
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(title)
                                .key("title")
                                .font(p.k.ty.ui_medium)
                                .color(p.text())
                                .a11y_hidden(),
                        )
                        .children(slot_keys(ui, p, slot).map(El::a11y_hidden)),
                )
                .child(
                    text(detail)
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control)
}

/// The shortcuts are free and Hex receives them: what else takes them,
/// each with where it's fixed, then a live press of each.
pub(super) fn shortcut(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let mac = ui.platform.caps.os == Os::Macos;
    let mut rows = Vec::new();
    match &flow.conflicts {
        None => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            if mac {
                tr("Looking for apps and macOS features on these shortcuts…")
            } else {
                tr("Looking for apps on these shortcuts…")
            },
            p.muted(),
            spinner(p, "looking"),
        )),
        Some(found) if found.is_empty() => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            if mac {
                tr("Nothing else on this Mac uses them")
            } else {
                tr("Nothing else on this computer uses them")
            },
            p.muted(),
            done_pill(p, "free", tr("Free")),
        )),
        Some(found) => {
            for (index, (slot, conflict)) in found.iter().enumerate() {
                let fix = conflict.fix.clone();
                let control = row()
                    .key("fixes")
                    .gap(8.0)
                    .center_y()
                    .child(push(
                        p,
                        "fix",
                        &conflict_label(&fix, &conflict.name),
                        index == 0 && conflict.taken,
                        move |ui: &mut Ui| setup::clear_conflict(ui, &fix),
                    ))
                    .children((*slot == HotkeySlot::Action && conflict.taken).then(|| {
                        Button::new(tr("Choose another…"))
                            .variant(ButtonVariant::Neutral)
                            .on_press_tx(setup::choose_another)
                            .build_push(p.k)
                            .key("choose-another")
                    }));
                let title = if conflict.taken {
                    tr_fill("{} takes it", &conflict.name)
                } else {
                    tr_fill("{} may take it", &conflict.name)
                };
                rows.push(row_of(
                    p,
                    &format!("conflict{index}"),
                    &glyphs::CIRCLE_X,
                    &title,
                    &format!("{}: {}", slot_name(*slot), conflict.description),
                    if conflict.taken {
                        p.danger()
                    } else {
                        p.warning()
                    },
                    control,
                ));
            }
        }
    }
    for slot in setup::TESTED {
        rows.push(probe_row(ui, flow, p, slot));
    }
    page(
        p,
        "shortcut",
        tr("Make the shortcuts yours"),
        lead(
            p,
            tr(
                "Nothing else may take them, and Hex has to receive them. Test each by pressing it once.",
            ),
        ),
        Some(rows_card(p, "shortcut-card", rows)),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions(p, page_done(ui, flow, Page::Shortcut), true),
    )
}
