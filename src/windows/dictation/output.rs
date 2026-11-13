//! The output worker's jobs: a dictation processed and pasted (or, for the
//! Action shortcut, handed to the assistant's question), and Paste last.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn process_dictation_job(
    mut job: Job,
    paster: &mut Result<WindowsPaster>,
    history: Option<&crate::history::History>,
    last_dictation: &Mutex<Option<String>>,
    mode_runtime: &RwLock<crate::dictation::modes::ModeRuntime>,
    transformations: &crate::personal_commands::TransformationClient,
) -> JobResult {
    let started = Instant::now();
    let printed = job.print.is_some();
    // The Action shortcut's question: processed like a paste, then handed
    // to its reply only.
    let asks = job.flavour == crate::dictation::DictationFlavour::Assistant;
    // The context read starts now so the UIA walk overlaps inference.
    let reads_page = !job.flavour.raw()
        && mode_runtime
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .reads_page();
    let pending_context = crate::windows::context::begin_capture(reads_page);
    let inference_started = Instant::now();
    let transcription = job.transcript.wait().map(|transcribed| {
        tracing::info!(
            source = transcribed.source.label(),
            finish_ms = transcribed.finish_ms,
            audio_ms = job.audio_ms,
            "transcribed Windows dictation"
        );
        transcribed.text
    });
    let inference_ms = inference_started.elapsed().as_millis() as u64;
    let context = pending_context.finish();
    let application = context.application.clone();
    let result = transcription.and_then(|raw_text| {
        if raw_text.is_empty() {
            return Err("transcription was empty".into());
        }
        let processed = if job.flavour.raw() {
            crate::dictation::processor::Processed::unchanged(raw_text.trim())
        } else {
            let mode_runtime = mode_runtime
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            mode_runtime.process(
                &raw_text,
                &job.language,
                &context,
                transformations,
                crate::dictation::processor::Source::Dictation,
            )
        };
        let text = processed.text.clone();
        if text.trim().is_empty() {
            return Err("text replacements produced empty output".into());
        }
        // A printed dictation is never pasted, and Paste Last skips it; nor
        // is a question, whatever became of its reply.
        if !printed && !asks {
            let paster = paster.as_mut().map_err(|error| format!("{error:#}"))?;
            // Copy dictation only, read as the output commits:
            // dictations go to the clipboard unpasted; Paste Last still pastes.
            if crate::features::copy_only() {
                paster
                    .copy_only(&text)
                    .map_err(|error| format!("{error:#}"))?;
            } else {
                paster
                    .paste(&text, job.flavour.terminal_paste())
                    .map_err(|error| format!("{error:#}"))?;
            }
            *last_dictation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(text.clone());
        }
        // The assistant's conversation keeps a question, not History.
        if let Some(history) = history.filter(|_| !asks) {
            let mut draft = crate::dictation::modes::history_draft(
                raw_text,
                &processed,
                application.clone(),
                job.audio_ms,
                inference_ms,
                started.elapsed().as_millis() as u64,
                job.recording.take(),
            );
            if printed {
                draft.kind = crate::history::HistoryKind::Printed;
            } else if job.flavour.raw() {
                draft.kind = crate::history::HistoryKind::Raw;
            }
            if let Err(error) = history.record(draft) {
                tracing::warn!(%error, "could not retain Windows dictation history");
            }
        }
        Ok(text)
    });
    let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    tracing::info!(
        audio_ms = job.audio_ms,
        latency_ms,
        "completed Windows dictation job"
    );
    if let Some(print) = job.print.take() {
        print.send(match &result {
            Ok(text) => PrintedTranscript::Text(text.clone()),
            Err(error) => PrintedTranscript::Failed(error.clone()),
        });
        return job_result(OutputKind::Printed, result, latency_ms);
    }
    job_result(OutputKind::Dictation, result, latency_ms)
}

pub(super) fn process_paste_last(
    paster: &mut Result<WindowsPaster>,
    last_dictation: &Mutex<Option<String>>,
) -> JobResult {
    let started = Instant::now();
    let result = last_dictation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .ok_or_else(|| "there is no completed dictation to paste".to_string())
        .and_then(|text| {
            paster
                .as_mut()
                .map_err(|error| format!("{error:#}"))?
                .paste(&text, false)
                .map_err(|error| format!("{error:#}"))?;
            Ok(text)
        });
    let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    tracing::info!(latency_ms, "completed Windows Paste Last job");
    job_result(OutputKind::Repaste, result, latency_ms)
}

pub(super) fn job_result(
    kind: OutputKind,
    result: Result<String, String>,
    latency_ms: u32,
) -> JobResult {
    match result {
        Ok(text) => JobResult {
            kind,
            text: Some(text),
            latency_ms,
            result: Ok(()),
        },
        Err(error) => JobResult {
            kind,
            text: None,
            latency_ms,
            result: Err(error),
        },
    }
}
