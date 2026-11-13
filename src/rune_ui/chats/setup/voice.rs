//! The Voice step's speech model and dictation try: what used to be the
//! voice first run's own screens (the model download, the microphone, the
//! practice dictation) as part of the one setup flow. On macOS with the
//! assistant on and its setup unfinished, a new install opens this flow
//! instead of the old first run ([`takes_first_run`]); the Microphone is a
//! row of Permissions, the model and the try are the Voice page's.
//!
//! The model downloads as the flow opens, so it runs while the person
//! reads, through the same service request the first run sent; the page
//! shows its progress and offers Try again after a failure. The Voice page
//! is done once the shortcuts work and the model is on the Mac; finishing
//! the flow records the voice onboarding as done too.

use super::super::super::Request;
use super::super::Ui;

/// Where the speech model stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::rune_ui) enum Model {
    /// On this Mac.
    Ready,
    /// Downloading, from zero through one.
    Downloading(f32),
    /// The last download failed.
    Failed,
    /// Not on this Mac and not downloading.
    Missing,
}

/// The selected speech model's state, as the service reports it.
pub(in crate::rune_ui) fn model(ui: &Ui) -> Model {
    let (installed, progress) = super::super::super::onboarding::progress(ui);
    if installed {
        Model::Ready
    } else if let Some(done) = progress.downloading {
        Model::Downloading(done)
    } else if progress.failed {
        Model::Failed
    } else {
        Model::Missing
    }
}

/// Whether the setup flow takes the voice first run's place: macOS (the
/// only OS with the Voice page), the assistant on, its setup unfinished.
pub(in crate::rune_ui) fn takes_first_run(ui: &Ui) -> bool {
    cfg!(target_os = "macos")
        && crate::assistant::enabled(&ui.settings)
        && !ui.settings.assistant.setup_done
}

/// Downloads the selected model when it is neither here nor on its way
/// (Download, Try again, and the flow opening).
pub(in crate::rune_ui) fn download(ui: &mut Ui) {
    if !matches!(model(ui), Model::Missing | Model::Failed) {
        return;
    }
    let selection = ui.settings.transcription.clone();
    if crate::transcription::models::validate(&selection).is_ok() {
        ui.send(Request::ChooseModel {
            model: selection.model,
            language: selection.language,
        });
    }
}

/// The try's field.
pub(in crate::rune_ui) fn practice(ui: &mut Ui) -> &mut rune::TextEdit {
    &mut ui.chats.setup.get_or_insert_with(Default::default).practice
}

/// Whether the try heard something: words landed in its field.
pub(in crate::rune_ui) fn practiced(ui: &Ui) -> bool {
    ui.chats
        .setup
        .as_ref()
        .is_some_and(|flow| !flow.practice.text.trim().is_empty())
}

/// Finishing the flow finishes the voice onboarding too: the old first run
/// doesn't come back at the next launch. The model, if it still
/// downloads, goes on in the service.
pub(in crate::rune_ui) fn finished(ui: &mut Ui) {
    if ui.live()
        && !crate::desktop::onboarding::completion_recorded()
        && let Err(error) = crate::desktop::onboarding::record_completion()
    {
        tracing::warn!(%error, "could not record onboarding completion");
    }
    ui.tour = None;
}
