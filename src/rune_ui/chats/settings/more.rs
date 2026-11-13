//! Assistant settings, round three: how freely the assistant acts (Ask,
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

/// "Listen" follow-up choices, in seconds; 0 turns conversation mode off.
const CONVERSATION: [u32; 4] = [0, 5, 8, 15];

const MODES: [Autonomy; 3] = [Autonomy::Ask, Autonomy::Smart, Autonomy::Yolo];

/// What a mode lets it do, in a line.
fn explain(mode: Autonomy) -> &'static str {
    match mode {
        Autonomy::Ask => {
            tr("Asks you before it sends, deletes, pays, shares, or publishes anything")
        }
        Autonomy::Smart => tr("Does what you asked for; anything else that matters asks first"),
        Autonomy::Yolo => tr(
            "Does everything without asking: no sandbox, the whole internet, every folder and tool (git, npm, Tailscale, …), and it sends, deletes, pays, and publishes when you ask. Only for tasks you trust",
        ),
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

/// A voice's name without its quality, and the quality as a word.
fn voice_label(name: &str) -> String {
    if let Some(preset) = crate::assistant::speech::neural::preset(name) {
        return format!("{} {preset} · {}", tr("Hex voice"), tr("Neural"));
    }
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
    voices.sort_by_key(|voice| {
        (
            crate::assistant::speech::neural::preset(voice).is_none(),
            std::cmp::Reverse(voices::quality(voice)),
        )
    });
    // Once Hex's own voice is here, only its presets are offered: the
    // system voices still read what it can't (another language, a
    // failure), without crowding the list.
    let hex_voices = voices
        .iter()
        .any(|voice| crate::assistant::speech::neural::preset(voice).is_some());
    if hex_voices {
        voices.retain(|voice| crate::assistant::speech::neural::preset(voice).is_some());
    }
    let offset = usize::from(!hex_voices);
    let mut labels = Vec::new();
    if !hex_voices {
        labels.push(tr("System voice").to_owned());
    }
    labels.extend(voices.iter().map(|voice| voice_label(voice)));
    let selected = ui
        .settings
        .assistant
        .voice
        .as_deref()
        .and_then(|voice| voices.iter().position(|known| known == voice))
        .map_or(0, |index| index + offset);
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
                .checked_sub(offset)
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

/// Hex's own voice, until it's downloaded: what it is, and Download (its
/// progress, or why it failed).
pub(super) fn hex_voice_row(ui: &Ui, p: Paint) -> Option<CardRow> {
    use crate::assistant::speech::neural::files;
    let installed = ui.chats.settings.voices.as_ref().is_some_and(|voices| {
        voices
            .iter()
            .any(|voice| voice.starts_with(crate::assistant::speech::neural::PREFIX))
    });
    if installed {
        return None;
    }
    let megabytes = (files::total_bytes() / 1_000_000).to_string();
    let (description, color) = match &ui.chats.settings.voice_download {
        Some(Err(error)) => (
            error.lines().next().unwrap_or_default().to_owned(),
            p.danger(),
        ),
        Some(Ok(bytes)) => (
            crate::desktop::i18n::tr_fill(
                "Downloading… {} %",
                &(bytes * 100 / files::total_bytes().max(1)).to_string(),
            ),
            p.muted(),
        ),
        None => (
            crate::desktop::i18n::tr_fill(
                "A natural voice that runs on this computer, in 31 languages (Supertonic 3, about {} MB)",
                &megabytes,
            ),
            p.muted(),
        ),
    };
    let downloading = matches!(ui.chats.settings.voice_download, Some(Ok(_)));
    let control = if downloading {
        text(tr("Downloading…")).color(p.muted())
    } else {
        small_button(p, "download-hex-voice", tr("Download"), |ui: &mut Ui, _| {
            ui.chats.settings.voice_download = Some(Ok(0));
            ui.chats.jobs.run(Job::DownloadVoice);
        })
    };
    Some(
        described_row(
            p,
            "assistant-hex-voice",
            tr("Hex voice"),
            Some((description, color)),
            control,
        )
        .into(),
    )
}

/// The two follow-up windows in one row: how long Hex listens for an
/// answer back after a spoken answer (`conversation_seconds`, "Listen"),
/// and how soon a hold continues that chat (`follow_up_seconds`,
/// "Continue chat").
pub(super) fn follow_up_row(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> CardRow {
    let assistant = &ui.settings.assistant;
    let (heard, heard_at) = super::with_saved(&CONVERSATION, assistant.conversation_seconds);
    let listen = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-conversation"),
        "assistant-conversation-control".to_owned(),
        tr("Listen"),
        heard
            .iter()
            .map(|seconds| super::seconds_label(*seconds))
            .collect(),
        heard_at,
        move |ui, chosen| {
            let seconds = heard[chosen];
            ui.update(|settings| settings.assistant.conversation_seconds = seconds);
        },
        layer,
    );
    let (held, held_at) = super::with_saved(&super::FOLLOW_UP, assistant.follow_up_seconds);
    let resume = quiet_select(
        ui,
        p,
        Menu::Pick("assistant-follow-up"),
        "assistant-follow-up-control".to_owned(),
        tr("Continue chat"),
        held.iter()
            .map(|seconds| super::seconds_label(*seconds))
            .collect(),
        held_at,
        move |ui, chosen| {
            let seconds = held[chosen];
            ui.update(|settings| settings.assistant.follow_up_seconds = seconds);
        },
        layer,
    );
    let controls = row()
        .key("assistant-follow-up-controls")
        .gap(8.0)
        .center_y()
        .child(text(tr("Listen")).color(p.muted()))
        .child(listen)
        .child(text(tr("Continue chat")).color(p.muted()).ml(8.0))
        .child(resume);
    setting_row(
        p,
        "assistant-follow-up",
        tr("Follow-ups"),
        Some(tr(
            "Listen: answer back without the shortcut, for a moment (paused while other audio plays). Continue chat: a hold this soon after an answer goes on with that chat",
        )),
        controls,
    )
    .into()
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
/// Clear…, then "Clear everything it knows" to confirm: empties the note
/// and saves it.
fn clear_memory_button(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings;
    if state.memory.text.trim().is_empty() {
        return rune::el().key("clear-memory");
    }
    if state.memory_clear_armed {
        Button::new(tr("Clear everything it knows"))
            .variant(ButtonVariant::Danger)
            .on_press(|ui: &mut Ui| {
                ui.chats.settings.memory_clear_armed = false;
                ui.chats.settings.memory.clear();
                save_memory(ui);
            })
            .build_small_push(p.k)
            .key("clear-memory")
    } else {
        Button::new(tr("Clear…"))
            .variant(ButtonVariant::Ghost)
            .on_press(|ui: &mut Ui| ui.chats.settings.memory_clear_armed = true)
            .build_small_push(p.k)
            .key("clear-memory")
    }
}

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
                    row()
                        .key("memory-actions")
                        .gap(8.0)
                        .center_y()
                        .child(
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
                        )
                        .child(clear_memory_button(ui, p)),
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
