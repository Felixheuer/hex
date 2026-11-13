//! The assistant's own switch (`assistant.enabled`, on by default): its row
//! at the top of Chats › Settings, and the calm card Chats shows while it's
//! off. Off, the Action shortcut, the overlays, the scheduler, and the menu
//! bar items are inert (`assistant::enabled`); the pane stays in the
//! sidebar so it can be turned back on.

use rune::{El, col, text};

use super::super::controls::{Paint, card, toggle_row};
use super::Ui;
use crate::desktop::i18n::{tr, tr_fill};

/// What the assistant does, under its name, with this computer's Action
/// shortcut ("Hold right ⌘", "Hold Right Ctrl").
fn about(ui: &Ui) -> String {
    let keys = ui
        .settings
        .assistant
        .action_hotkey
        .as_ref()
        .map(|binding| binding.keycaps().join(" "))
        .filter(|keys| !keys.is_empty());
    match keys {
        _ if cfg!(target_os = "macos")
            && keys
                .as_deref()
                .is_none_or(|keys| keys == "Right ⌘" || keys == "R⌘") =>
        {
            tr("Hold right ⌘ to ask, double-tap for your tasks").to_owned()
        }
        Some(keys) => tr_fill("Hold {} to ask, double-tap for your tasks", &keys),
        None => tr("Set an Action shortcut in General to ask by voice").to_owned(),
    }
}

/// Turns the assistant on or off. On, an Action shortcut another shortcut
/// took meanwhile is unbound rather than left dead, and an unfinished setup
/// opens its onboarding.
pub(super) fn set(ui: &mut Ui, on: bool) {
    ui.update(|settings| {
        settings.assistant.enabled = on;
        settings.repair_action_shortcut();
    });
    if on && !ui.settings.assistant.setup_done {
        super::open_setup(ui);
    }
}

/// Settings' first card: the assistant's on/off row.
pub(super) fn settings_card(ui: &Ui, p: Paint) -> El {
    let on = crate::assistant::enabled(&ui.settings);
    let about = about(ui);
    card(
        p,
        "assistant-switch",
        vec![toggle_row(
            p,
            "assistant-enabled",
            tr("Assistant"),
            about.as_str(),
            on,
            move |ui: &mut Ui| set(ui, !on),
        )],
    )
}

/// The Chats view while the assistant is off: what it is, and Turn on.
pub(super) fn off_card(ui: &Ui, p: Paint) -> El {
    use super::super::onboarding_kit::{LEAD, title_font};
    col()
        .key("assistant-off")
        .grow(1.0)
        .shrink()
        .center()
        .gap(10.0)
        .pb(40.0)
        .child(
            text(tr("The assistant is off"))
                .key("title")
                .role("heading")
                .level(2)
                .font(title_font(p))
                .color(p.text()),
        )
        .child(
            text(about(ui))
                .key("detail")
                .font(LEAD)
                .color(p.muted())
                .align(rune::Align::Center)
                .wrap()
                .max_w(420.0),
        )
        .child(
            rune_kit::Button::new(tr("Turn on"))
                .variant(rune_kit::ButtonVariant::Contrast)
                .on_press(|ui: &mut Ui| set(ui, true))
                .build(p.k)
                .key("turn-on-assistant")
                .mt(8.0),
        )
}
