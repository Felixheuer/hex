//! The onboarding's Voice page: the speech model and the dictation try
//! (`voice`), then what else takes the tested shortcuts, each with where
//! it's fixed, and a live press of each.

use rune::{El, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, glyphs};

use super::super::super::controls::{Paint, PushSize};
use super::super::super::onboarding_kit::keys;
use super::super::Ui;
use super::super::setup::{self, Page, ProbeState, SetupFlow, page_done};
use super::super::shortcut::{ConflictFix, Probe};
use super::{
    actions, done_pill, lead, note, page, push, row_of, rows_card, section_label, spinner,
    waiting_pill,
};
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
fn probe_words(found: &Probe, taker: Option<&str>) -> (String, bool) {
    match found {
        Probe::Works(held) => (
            tr_fill(
                "Agent-Desk received it (held {})",
                &format!("{:.1} s", held.as_secs_f32()),
            ),
            true,
        ),
        Probe::Swallowed => (
            match taker {
                Some(name) => tr_fill("{} took the keys before Agent-Desk could", name),
                None => tr("Another app or macOS took the keys before Agent-Desk could").into(),
            },
            false,
        ),
        Probe::NothingReceived => (
            tr("Nothing reached Agent-Desk. Another app or macOS may take it").into(),
            false,
        ),
        Probe::WrongKeys(saw) => (tr_fill("Agent-Desk saw {} instead", saw), false),
        Probe::NoRelease => (tr("Agent-Desk never saw the keys let go").into(), false),
        Probe::Unavailable(why) => (why.clone(), false),
    }
}

/// One shortcut's live press test: Test, a waiting pill while it listens,
/// then what reached Agent-Desk.
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
            let (words, works) = probe_words(found, taker);
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

/// The shortcuts are free and Agent-Desk receives them: what else takes them,
/// each with where it's fixed, then a live press of each.
pub(super) fn shortcut(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let mut rows = Vec::new();
    match &flow.conflicts {
        None => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            tr("Looking for apps and macOS features on these shortcuts…"),
            p.muted(),
            spinner(p, "looking"),
        )),
        Some(found) if found.is_empty() => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            tr("Nothing else on this Mac uses them"),
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
    let card = rune::col()
        .key("voice-cards")
        .gap(20.0)
        .child(super::voice::section(ui, flow, p))
        .child(
            col()
                .key("shortcut-section")
                .gap(8.0)
                .child(section_label(p, "shortcut-label", tr("Shortcuts")))
                .child(rows_card(p, "shortcut-card", rows)),
        );
    page(
        p,
        "shortcut",
        tr("Your voice"),
        lead(
            p,
            tr(
                "Agent-Desk understands you right on this Mac: its speech model downloads once. Nothing else may take your shortcuts.",
            ),
        ),
        Some(card),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions(p, page_done(ui, flow, Page::Shortcut), true),
    )
}
