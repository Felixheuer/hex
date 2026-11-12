//! Choose the website before opening the dedicated sign-in browser.
//! Saved domains describe browser data; only each site can validate its login.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};

use super::super::controls::{Paint, PushSize};
use super::{Job, Ui};
use crate::desktop::i18n::{tr, tr_fill};

pub(super) fn open(ui: &mut Ui) {
    ui.chats.settings.sign_in_editor = true;
    ui.chats.settings.sign_in_error = None;
}

pub(super) fn launch(ui: &mut Ui) {
    if ui.chats.settings.sign_in_busy {
        return;
    }
    match crate::assistant::browser::sign_in_url(&ui.chats.settings.site_address.text) {
        Ok(url) => {
            ui.chats.settings.sign_in_busy = true;
            ui.chats.settings.sign_in_error = None;
            ui.chats.jobs.run(Job::SignIn(Some(url)));
        }
        Err(error) => ui.chats.settings.sign_in_error = Some(tr(error).into()),
    }
}

pub(super) fn save(ui: &mut Ui) {
    if ui.chats.settings.sign_in_busy {
        return;
    }
    ui.chats.settings.sign_in_busy = true;
    ui.chats.jobs.run(Job::FinishSignIn);
}

pub(super) fn summary(ui: &Ui) -> String {
    if ui.chats.settings.saved_sites.is_empty() {
        tr("No website data saved yet").into()
    } else {
        tr_fill(
            "Saved browser data: {}",
            &ui.chats.settings.saved_sites.join(", "),
        )
    }
}

pub(super) fn editor(ui: &Ui, p: Paint) -> Option<El> {
    let state = &ui.chats.settings;
    if !state.sign_in_editor {
        return None;
    }
    let open = Button::new(if state.sign_in_busy && !state.signing_in {
        tr("Opening browser…")
    } else {
        tr("Open website")
    })
    .variant(ButtonVariant::Neutral)
    .disabled(state.sign_in_busy || state.site_address.text.trim().is_empty())
    .on_press(launch)
    .build_push(p.k)
    .key("open-sign-in-site");
    let save = state.signing_in.then(|| {
        Button::new(if state.sign_in_busy {
            tr("Saving…")
        } else {
            tr("Save browser data")
        })
        .disabled(state.sign_in_busy)
        .on_press(save)
        .build_push(p.k)
        .key("save-site-sign-in")
    });
    Some(col().key("site-sign-in-editor").w_full().gap(12.0)
        .child(text(tr("Choose a website, sign in in the browser window, then save it here. This step is optional."))
            .key("instructions").font(p.k.ty.small).color(p.muted()).wrap())
        .child(row().key("website-row").gap(8.0).center_y()
            .child(TextInput::new(&state.site_address).label(tr("Website"))
                .placeholder("mail.google.com")
                .width(None).on_edit(|ui: &mut Ui| &mut ui.chats.settings.site_address)
                .on_submit(|ui: &mut Ui, _| launch(ui)).build(p.k)
                .key("sign-in-website").grow(1.0).shrink())
            .child(open))
        .children(save)
        .child(text(tr("Saved data is reused by web tasks. Each website checks whether you are still signed in.")).key("saved-sites").role("status")
            .font(p.k.ty.small).color(p.muted()).wrap())
        .children(state.sign_in_error.as_ref().map(|error| text(error.clone())
            .key("sign-in-error").role("alert").font(p.k.ty.small).color(p.danger()).wrap())))
}
