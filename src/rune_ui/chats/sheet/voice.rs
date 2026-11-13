//! The Voice page's speech-model card (`setup::voice`): the model with its
//! download's progress, Try again after a failure, and the optional
//! dictation try, a field the person's own dictation lands in. The old
//! first run's model, microphone, and demo screens live on here, in the
//! setup's card and rows.

use rune::{El, col, el, row};

use super::super::super::controls::Paint;
use super::super::super::icons::MIC;
use super::super::Ui;
use super::super::setup::SetupFlow;
use super::super::setup::voice::{self, Model};
use super::{done_pill, push, rows_card, section_label, status_row};
use crate::desktop::i18n::{tr, tr_fill};

/// How wide the download's bar is.
const BAR_W: f32 = 120.0;

/// The download's bar, filled to `done`.
fn bar(p: Paint, done: f32) -> El {
    let k = p.k;
    el().key("download-bar")
        .role("progressbar")
        .label(tr("Speech model"))
        .a11y_value(format!("{} %", (done * 100.0).round()))
        .size(BAR_W, 6.0)
        .radius(3.0)
        .fill(k.c.border_base)
        .child(
            el().key("download-fill")
                .size((BAR_W * done.clamp(0.02, 1.0)).round(), 6.0)
                .radius(3.0)
                .fill(p.accent())
                .motion(rune::Spring::new(0.3, 0.0)),
        )
}

/// The speech model's row: ready, its download's progress, or Try again.
fn model_row(ui: &Ui, p: Paint) -> El {
    let name = crate::transcription::models::definition(ui.settings.transcription.model).name;
    let title = tr("Speech model");
    match voice::model(ui) {
        Model::Ready => status_row(
            p,
            "speech-model",
            &MIC,
            true,
            title,
            &tr_fill("{} · ready on this Mac", name),
            done_pill(p, "model-ready", tr("Ready")),
        ),
        Model::Downloading(done) => status_row(
            p,
            "speech-model",
            &MIC,
            false,
            title,
            &tr_fill(
                "Downloading… {}",
                &format!("{name} · {} %", (done * 100.0).round()),
            ),
            bar(p, done),
        ),
        Model::Failed => status_row(
            p,
            "speech-model",
            &MIC,
            false,
            title,
            tr("The download stopped. Check the connection and try again"),
            push(p, "model-retry", tr("Try again"), true, voice::download),
        ),
        Model::Missing => status_row(
            p,
            "speech-model",
            &MIC,
            false,
            title,
            &tr_fill("{} · not downloaded yet", name),
            push(p, "model-download", tr("Download"), true, voice::download),
        ),
    }
}

/// The optional try: a field to dictate into, once the model is here.
fn practice_row(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let k = p.k;
    let ready = voice::model(ui) == Model::Ready;
    let keys =
        super::super::super::settings::shortcut_caps(&ui.settings.dictation_hotkey, false).join("");
    let detail = if ready {
        tr_fill("Optional: click the field, hold {} and speak", &keys)
    } else {
        tr("Once the speech model is here").to_owned()
    };
    let heard = voice::practiced(ui);
    let control = if heard {
        done_pill(p, "practice-heard", tr("Works"))
    } else {
        el().key("practice-waiting").size(1.0, 1.0)
    };
    let top = status_row(
        p,
        "practice",
        &rune_kit::glyphs::KEYBOARD,
        heard,
        tr("Try dictation"),
        &detail,
        control,
    );
    let ink = k.c.text_base;
    let field = rune::text_input(&flow.practice, k.ty.ui)
        .key("practice-input")
        .w_full()
        .color(ink)
        .placeholder(tr("Your words land here"), k.c.text_faint)
        .caret(ink, k.selection())
        .label(tr("Try dictation"))
        .on_edit(voice::practice);
    col().key("practice-group").w_full().children([
        top,
        row()
            .key("practice-field-row")
            .w_full()
            .px(16.0)
            .pb(12.0)
            .child(
                el().key("practice-box")
                    .w_full()
                    .min_h(40.0)
                    .px(10.0)
                    .py(9.0)
                    .radius(8.0)
                    .fill(k.c.layer_01)
                    .border(
                        p.hair,
                        if heard {
                            k.alpha(p.success(), 0.6)
                        } else {
                            k.c.border_base
                        },
                    )
                    .opacity(if ready { 1.0 } else { 0.55 })
                    .role("textbox")
                    .child(field),
            ),
    ])
}

/// The Voice page's first section: the speech model and the try, under
/// their label.
pub(super) fn section(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    col()
        .key("voice-section")
        .gap(8.0)
        .child(section_label(p, "voice-label", tr("Dictate")))
        .child(rows_card(
            p,
            "voice-card",
            vec![model_row(ui, p), practice_row(ui, flow, p)],
        ))
}
