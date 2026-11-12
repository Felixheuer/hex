//! Chats › Settings, round three: how freely the assistant acts (Ask,
//! Smart, YOLO), what it remembers about you, its voice with quality and a
//! better one a click away, listening after an answer and interrupting it
//! by speaking, and notifications.

use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, TextArea};

use super::super::super::Menu;
use super::super::super::controls::{
    CardRow, DESCRIPTION, Paint, PushSize, described_row, section, segmented_tx, setting_row,
    toggle_row,
};
use super::super::super::general::quiet_select;
use super::super::super::lists::small_button;
use super::super::bridge::{memory, voices};
use super::super::{Job, Ui};
use crate::assistant::autonomy::Autonomy;
use crate::desktop::i18n::tr;

/// "Keep listening after an answer" choices, in seconds.
const CONVERSATION: [u32; 4] = [0, 5, 8, 15];

const MODES: [Autonomy; 3] = [Autonomy::Ask, Autonomy::Smart, Autonomy::Yolo];

/// What a mode lets it do, in a line.
fn explain(mode: Autonomy) -> &'static str {
    match mode {
        Autonomy::Ask => {
            tr("Asks you before it sends, deletes, pays, shares, or publishes anything")
        }
        Autonomy::Smart => tr("Does what you asked for; anything else that matters asks first"),
        Autonomy::Yolo => tr("Does everything without asking. Use it only for tasks you trust"),
    }
}

/// Ask / Smart · recommended / YOLO, its explanation, and the note that
/// text it reads never grants permission. Onboarding and Settings.
pub(in crate::rune_ui) fn autonomy_choice(ui: &Ui, p: Paint) -> El {
    let current = ui.settings.assistant.autonomy;
    let labels = [tr("Ask"), tr("Smart"), tr("YOLO")];
    let selected = MODES.iter().position(|mode| *mode == current).unwrap_or(1);
    col()
        .key("autonomy")
        .gap(8.0)
        .child(segmented_tx(
            p,
            "autonomy-mode",
            &labels,
            selected,
            |ui: &mut Ui, index: usize, tx: &mut Tx| {
                tx.instant();
                ui.update(|settings| settings.assistant.autonomy = MODES[index.min(2)]);
            },
        ))
        .child(
            text(if current == Autonomy::Smart {
                crate::desktop::i18n::tr_fill("Recommended: {}", explain(current))
            } else {
                explain(current).to_owned()
            })
            .key("explain")
            .role("status")
            .font(DESCRIPTION)
            .color(if current == Autonomy::Yolo {
                p.warning()
            } else {
                p.text()
            })
            .wrap(),
        )
        .child(
            text(tr(
                "Text in websites and emails never counts as your permission, whatever it says.",
            ))
            .key("note")
            .font(p.k.ty.small)
            .color(p.muted())
            .wrap(),
        )
}

pub(in crate::rune_ui) fn autonomy_section(ui: &Ui, p: Paint) -> El {
    section(
        p,
        "assistant-autonomy",
        tr("How freely it acts"),
        vec![
            row()
                .key("autonomy-row")
                .px(16.0)
                .py(12.0)
                .child(autonomy_choice(ui, p).grow(1.0)),
        ],
    )
}

/// A voice's name without its quality, and the quality as a word.
fn voice_label(name: &str) -> String {
    let quality = match voices::quality(name) {
        voices::Quality::Premium => tr("Premium"),
        voices::Quality::Enhanced => tr("Enhanced"),
        voices::Quality::Default => tr("Default"),
        voices::Quality::Personal => tr("Personal Voice"),
    };
    let bare = name.replace(" (Premium)", "").replace(" (Enhanced)", "");
    format!("{bare} · {quality}")
}

/// The voice, best first with its quality, and Test voice.
pub(super) fn voice_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let mut voices = ui.chats.settings.voices.clone().unwrap_or_default();
    voices.sort_by_key(|voice| std::cmp::Reverse(voices::quality(voice)));
    let mut labels = vec![tr("System voice").to_owned()];
    labels.extend(voices.iter().map(|voice| voice_label(voice)));
    let selected = ui
        .settings
        .assistant
        .voice
        .as_deref()
        .and_then(|voice| voices.iter().position(|known| known == voice))
        .map_or(0, |index| index + 1);
    let picker = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-voice"),
        "assistant-voice-control".to_owned(),
        tr("Voice"),
        labels,
        selected,
        move |ui, chosen| {
            let voice = chosen
                .checked_sub(1)
                .and_then(|index| voices.get(index).cloned());
            ui.update(|settings| settings.assistant.voice = voice);
        },
        layer,
    );
    setting_row(
        p,
        "assistant-voice",
        tr("Voice"),
        None,
        row()
            .key("voice-controls")
            .gap(8.0)
            .center_y()
            .child(picker)
            .child(small_button(
                p,
                "test-voice",
                tr("Test voice"),
                |ui: &mut Ui, _| {
                    let voice = ui.settings.assistant.voice.clone();
                    ui.chats.jobs.run(Job::Speak(voice));
                },
            )),
    )
}

/// "Get a better voice", when none installed sounds natural.
pub(super) fn better_voice_row(ui: &Ui, p: Paint) -> Option<CardRow> {
    let installed = ui.chats.settings.voices.as_ref()?;
    voices::better_voice_available(installed)?;
    Some(
        described_row(
            p,
            "assistant-better-voice",
            tr("Get a better voice"),
            Some((
                tr("Premium voices sound far more natural. Download one in System Settings")
                    .to_owned(),
                p.muted(),
            )),
            small_button(
                p,
                "get-better-voice",
                tr("Open Settings…"),
                |ui: &mut Ui, _| {
                    if ui.chats.live() {
                        voices::open_voice_settings();
                    }
                },
            ),
        )
        .into(),
    )
}

/// Keep listening after an answer, and interrupt by speaking.
pub(super) fn conversation_rows(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<CardRow> {
    let saved = ui.settings.assistant.conversation_seconds;
    let (values, selected) = super::with_saved(&CONVERSATION, saved);
    let labels = values
        .iter()
        .map(|seconds| match seconds {
            0 => tr("Off").to_owned(),
            _ => crate::desktop::i18n::tr_fill("{} s", &seconds.to_string()),
        })
        .collect();
    let listen = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-conversation"),
        "assistant-conversation-control".to_owned(),
        tr("Keep listening after an answer"),
        labels,
        selected,
        move |ui, chosen| {
            let seconds = values[chosen];
            ui.update(|settings| settings.assistant.conversation_seconds = seconds);
        },
        layer,
    );
    vec![
        setting_row(
            p,
            "assistant-conversation",
            tr("Keep listening after an answer"),
            Some(tr("Answer back without the shortcut, for a moment")),
            listen,
        )
        .into(),
        toggle_row(
            p,
            "assistant-barge-in",
            tr("Interrupt by speaking"),
            tr("Talk over an answer to stop it. Works best with headphones"),
            ui.settings.assistant.barge_in,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.barge_in ^= true);
            },
        )
        .into(),
    ]
}

pub(super) fn notifications_row(ui: &Ui, p: Paint) -> CardRow {
    toggle_row(
        p,
        "assistant-notifications",
        tr("Notifications"),
        tr("When a job finishes, fails, or asks you something"),
        ui.settings.assistant.notifications,
        |ui: &mut Ui| {
            ui.update(|settings| settings.assistant.notifications ^= true);
        },
    )
    .into()
}

/// Loads what Hex knows, for the editor.
pub(super) fn opened(ui: &mut Ui) {
    ui.chats.settings.memory = rune::TextEdit::new(memory::read());
    ui.chats.settings.memory_saved = true;
}

fn save_memory(ui: &mut Ui) {
    let text = ui.chats.settings.memory.text.clone();
    match memory::write(&text) {
        Ok(()) => ui.chats.settings.memory_saved = true,
        Err(error) => ui.error = Some(format!("{error:#}")),
    }
}

/// "What Hex knows about you": the switch, and the facts as text.
pub(in crate::rune_ui) fn memory_section(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings;
    let on = ui.settings.assistant.memory;
    let mut rows: Vec<CardRow> = vec![
        toggle_row(
            p,
            "assistant-memory",
            tr("Remember things about me"),
            tr("When you tell it something worth keeping, it adds it here. Edit or delete anything"),
            on,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.memory ^= true);
            },
        )
        .into(),
    ];
    if on {
        let editor = TextArea::new(&state.memory)
            .placeholder(tr(
                "For example: I work at Acme as a designer. My manager is Sam.",
            ))
            .label(tr("What Hex knows about you"))
            .width(None)
            .auto_grow(3, 12)
            .on_edit(|ui: &mut Ui| {
                ui.chats.settings.memory_saved = false;
                &mut ui.chats.settings.memory
            })
            .build(p.k)
            .key("memory-editor");
        rows.push(
            col()
                .key("memory-edit")
                .px(16.0)
                .py(12.0)
                .gap(8.0)
                .child(editor)
                .child(
                    row().key("memory-actions").gap(8.0).center_y().child(
                        Button::new(if state.memory_saved {
                            tr("Saved")
                        } else {
                            tr("Save")
                        })
                        .variant(ButtonVariant::Neutral)
                        .disabled(state.memory_saved)
                        .on_press(save_memory)
                        .build_small_push(p.k)
                        .key("save-memory"),
                    ),
                )
                .into(),
        );
    }
    section(
        p,
        "assistant-memory-section",
        tr("What Hex knows about you"),
        rows,
    )
}
