//! What a finished transcript goes through on Linux: the modes that
//! process it, the paste, and the History that keeps it.

use std::sync::{Arc, RwLock};
use std::time::Instant;

use color_eyre::Result;
use color_eyre::eyre::eyre;

use crate::dictation::DictationFlavour;
use crate::linux::paste::LinuxPaster;

/// What happens to a transcript before and after it is pasted: the modes
/// that process it and the history that keeps it. The host shares it
/// with the listener so saved settings apply without a restart.
#[derive(Clone)]
pub(crate) struct DictationOutput {
    pub(crate) history: Option<crate::history::History>,
    pub(crate) modes: Arc<RwLock<crate::dictation::modes::ModeRuntime>>,
    pub(crate) transformations: Arc<crate::personal_commands::TransformationClient>,
}

impl DictationOutput {
    pub(crate) fn from_settings(
        settings: &crate::app_settings::AppSettings,
        history: Option<crate::history::History>,
    ) -> Self {
        Self {
            history,
            modes: Arc::new(RwLock::new(
                crate::dictation::modes::ModeRuntime::from_settings(settings),
            )),
            transformations: Arc::new(crate::personal_commands::TransformationClient::default()),
        }
    }

    /// Reuses the mode runtime and transformation client the local API's
    /// `process=global` reads, so both see the same settings and personal
    /// commands host.
    pub(crate) fn sharing(mut self, processing: crate::dictation::modes::ProcessingState) -> Self {
        self.modes = processing.modes;
        self.transformations = processing.transformations;
        self
    }

    /// Processes (unless raw) and pastes one transcript, or only
    /// processes it when there is no paster (a printing stop), then records
    /// it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn deliver(
        &self,
        raw_text: String,
        flavour: crate::dictation::DictationFlavour,
        language: &str,
        paster: Option<&mut LinuxPaster>,
        audio_ms: u64,
        inference_ms: u64,
        started: Instant,
        recording: Option<crate::history::Recording>,
        typing: Option<crate::linux::live_typing::Session>,
    ) -> Result<String> {
        let modes = self
            .modes
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let context = crate::linux::context::capture(!flavour.raw() && modes.reads_page());
        let processed = if flavour.raw() {
            crate::dictation::processor::Processed::unchanged(raw_text.trim())
        } else {
            modes.process(
                &raw_text,
                language,
                &context,
                &self.transformations,
                crate::dictation::processor::Source::Dictation,
            )
        };
        let printed = paster.is_none();
        // Live typing inserts the final text through the input method; a
        // printed or copy-only dictation clears its preedit and takes the
        // usual path instead.
        let inserted = match typing {
            Some(session) if !printed && !crate::features::copy_only() => {
                session.finish(&processed.text)
            }
            Some(session) => {
                session.cancel();
                false
            }
            None => false,
        };
        if processed.text.trim().is_empty() {
            return Err(eyre!("text replacements produced empty output"));
        }
        if let Some(paster) = paster
            && !inserted
        {
            paster.paste(&processed.text, flavour.terminal_paste())?;
        }
        // A question to the assistant is never kept: its conversation is.
        if let Some(history) = &self.history
            && flavour != DictationFlavour::Assistant
        {
            let mut draft = crate::dictation::modes::history_draft(
                raw_text,
                &processed,
                context.application,
                audio_ms,
                inference_ms,
                started.elapsed().as_millis() as u64,
                recording,
            );
            if printed {
                draft.kind = crate::history::HistoryKind::Printed;
            } else if flavour.raw() {
                draft.kind = crate::history::HistoryKind::Raw;
            }
            if let Err(error) = history.record(draft) {
                tracing::warn!(%error, "could not retain Linux dictation history");
            }
        }
        Ok(processed.text)
    }
}
