//! Assistant settings › „Aufs iPhone, wenn ich weg bin“: the person's own
//! number or Apple ID, when they count as away, a test message, and the two
//! permissions the channel needs (Messages automation to send, Full Disk
//! Access to read replies). The channel itself is `assistant::phone`.

use rune::{El, TextEdit, row};
use rune_kit::TextInput;

use super::super::super::Menu;
use super::super::super::controls::{
    CardRow, Paint, note, section as card, setting_row, toggle_row,
};
use super::super::super::general::quiet_select;
use super::super::super::lists::small_button;
use super::super::Ui;
use crate::assistant::phone;
use crate::assistant::phone::incoming::{self, Access};
use crate::desktop::automation::AutomationState;
use crate::desktop::i18n::{tr, tr_fill};

/// Idle limits offered, in minutes; 0: only the switch, the lock, or saying so.
const AWAY_AFTER: [u32; 6] = [0, 5, 10, 15, 30, 60];

/// The section's field and what its rows show.
#[derive(Default)]
pub(in crate::rune_ui) struct PhoneState {
    /// The number or Apple ID as typed.
    pub(in crate::rune_ui) handle: TextEdit,
    /// What was typed is neither a phone number nor an e-mail address.
    invalid: bool,
    /// Whether the Messages database is readable, read when Settings opens.
    access: Option<Access>,
    /// The test message, once sent.
    tested: Option<String>,
}

/// Settings opened: the saved handle shows, and Full Disk Access is read.
pub(super) fn opened(ui: &mut Ui) {
    let saved = ui
        .settings
        .assistant
        .phone_handle
        .clone()
        .unwrap_or_default();
    let state = &mut ui.chats.settings.phone;
    state.handle = TextEdit::new(saved);
    state.invalid = false;
    state.tested = None;
    state.access = Some(incoming::access());
}

fn save_handle(ui: &mut Ui) {
    let typed = ui.chats.settings.phone.handle.text.trim().to_owned();
    if typed.is_empty() {
        ui.chats.settings.phone.invalid = false;
        ui.update(|settings| settings.assistant.phone_handle = None);
        return;
    }
    let ok = incoming::clean_handle(&typed).is_some();
    ui.chats.settings.phone.invalid = !ok;
    if ok {
        ui.update(|settings| settings.assistant.phone_handle = Some(typed));
    }
}

pub(super) fn section(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let assistant = &ui.settings.assistant;
    let state = &ui.chats.settings.phone;
    let mut rows: Vec<CardRow> = vec![
        toggle_row(
            p,
            "assistant-phone",
            tr("Send to my iPhone when I'm away"),
            tr("Questions, approvals, failures and finished jobs go out by iMessage; your replies come back as if typed here"),
            assistant.phone_enabled,
            |ui: &mut Ui| {
                let on = !ui.settings.assistant.phone_enabled;
                ui.update(|settings| settings.assistant.phone_enabled = on);
                if on {
                    phone::ask_messages();
                    phone::start();
                }
            },
        )
        .into(),
    ];
    if !assistant.phone_enabled {
        return card(p, "assistant-phone-section", tr("On my iPhone"), rows);
    }
    rows.push(handle_row(ui, p).into());
    if state.invalid {
        rows.push(
            note(
                "phone-invalid",
                tr("Enter your own phone number (+49 …) or Apple ID e-mail"),
                p.danger(),
            )
            .into(),
        );
    }
    rows.push(away_row(ui, p, layer).into());
    rows.push(
        toggle_row(
            p,
            "assistant-phone-away-now",
            tr("I'm away now"),
            tr("Also when the screen is locked, or after you say „ich gehe jetzt …“"),
            phone::away::manual(),
            |_: &mut Ui| {
                phone::away::set_manual(!phone::away::manual());
                phone::start();
            },
        )
        .into(),
    );
    rows.push(messages_row(p).into());
    rows.push(disk_row(state, p).into());
    rows.push(test_row(p).into());
    if let Some(sent) = &state.tested {
        rows.push(note("phone-test-text", sent, p.muted()).into());
    }
    card(p, "assistant-phone-section", tr("On my iPhone"), rows)
}

fn handle_row(ui: &Ui, p: Paint) -> El {
    row()
        .key("phone-handle")
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .child(
            TextInput::new(&ui.chats.settings.phone.handle)
                .placeholder(tr("Your phone number or Apple ID"))
                .label(tr("Your phone number or Apple ID"))
                .width(None)
                .on_edit(|ui: &mut Ui| &mut ui.chats.settings.phone.handle)
                .on_submit(|ui: &mut Ui, _| save_handle(ui))
                .build(p.k)
                .key("phone-handle-field")
                .grow(1.0)
                .shrink(),
        )
        .child(small_button(
            p,
            "phone-handle-save",
            tr("Save"),
            |ui: &mut Ui, _| save_handle(ui),
        ))
}

fn away_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let (values, selected) =
        super::with_saved(&AWAY_AFTER, ui.settings.assistant.phone_away_minutes);
    let select = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-phone-away"),
        "assistant-phone-away-control".to_owned(),
        tr("Away after"),
        values
            .iter()
            .map(|minutes| match minutes {
                0 => tr("Only when locked or told").to_owned(),
                _ => tr_fill("{} min without input", &minutes.to_string()),
            })
            .collect(),
        selected,
        move |ui, chosen| {
            let minutes = values[chosen];
            ui.update(|settings| settings.assistant.phone_away_minutes = minutes);
        },
        layer,
    );
    setting_row(
        p,
        "assistant-phone-away-after",
        tr("Away after"),
        None,
        select,
    )
}

fn messages_row(p: Paint) -> El {
    let (copy, ok) = match phone::messages_permission() {
        Some(AutomationState::Granted) => (tr("Hex may send with Messages"), true),
        Some(AutomationState::Denied) => (
            tr("Turned off in System Settings › Privacy & Security › Automation"),
            false,
        ),
        _ => (tr("Hex asks once to send with Messages"), false),
    };
    let control = if ok {
        row()
    } else {
        small_button(p, "phone-messages-allow", tr("Allow"), |_: &mut Ui, _| {
            match phone::messages_permission() {
                Some(AutomationState::Denied) => crate::desktop::automation::open_settings(),
                _ => phone::ask_messages(),
            }
        })
    };
    setting_row(p, "phone-messages", tr("Messages"), Some(copy), control)
}

fn disk_row(state: &PhoneState, p: Paint) -> El {
    let (copy, ok) = match state.access {
        Some(Access::Granted) => (tr("Hex reads your replies"), true),
        Some(Access::Missing) => (tr("Messages isn't set up on this Mac"), true),
        _ => (
            tr("Needed to read your replies: turn Hex on under Full Disk Access"),
            false,
        ),
    };
    let control = if ok {
        row()
    } else {
        small_button(p, "phone-disk-open", tr("Open"), |ui: &mut Ui, _| {
            incoming::open_full_disk_access();
            ui.chats.settings.phone.access = None;
        })
    };
    setting_row(p, "phone-disk", tr("Full Disk Access"), Some(copy), control)
}

fn test_row(p: Paint) -> El {
    setting_row(
        p,
        "phone-test",
        tr("Test message"),
        Some(tr("Sends one iMessage to the number above")),
        small_button(p, "phone-test-send", tr("Send test"), |ui: &mut Ui, _| {
            let sent = phone::send_test(&ui.settings.assistant);
            let state = &mut ui.chats.settings.phone;
            state.invalid = !sent;
            state.tested = sent.then(|| tr_fill("Sent: {}", &phone::test_message()));
        }),
    )
}
