//! Windows' own speech engine (`Windows.Media.SpeechSynthesis`, the voices
//! in Settings › Time & language › Speech), in process: each sentence comes
//! back as a WAV stream, so no program starts per sentence.

use std::path::Path;
use std::time::Duration;

use windows::Media::SpeechSynthesis::SpeechSynthesizer;
use windows::Storage::Streams::DataReader;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::HSTRING;

use super::super::voices::{self, Listed, Quality, Voice};
use super::{Job, Plan};

/// A little brisker than the engine's 1.0, as people talk to an assistant.
const SPEAKING_RATE: f64 = 1.1;

/// The sentence's audio, made now (on the speech worker's thread).
pub fn plan(job: &Job, _file: &Path) -> Plan {
    match synthesize(&job.text, job.voice.as_deref()) {
        Ok(bytes) => Plan::Audio(Some(bytes)),
        Err(error) => {
            tracing::warn!(%error, "Windows speech could not read a sentence");
            Plan::Audio(None)
        }
    }
}

/// The installed Windows voices ("Microsoft Katja", de-DE).
pub fn voices() -> Vec<Voice> {
    match listed() {
        Ok(listed) => voices::named(listed),
        Err(error) => {
            tracing::warn!(%error, "could not list the Windows voices");
            Vec::new()
        }
    }
}

fn listed() -> windows::core::Result<Vec<Listed>> {
    com();
    let mut listed = Vec::new();
    for voice in SpeechSynthesizer::AllVoices()? {
        let name = voice.DisplayName()?.to_string();
        listed.push(Listed {
            id: voice.Id()?.to_string(),
            name: name
                .strip_prefix("Microsoft ")
                .unwrap_or(&name)
                .trim()
                .to_owned(),
            locale: voice.Language()?.to_string(),
            quality: Quality::Default,
        });
    }
    Ok(listed)
}

fn synthesize(text: &str, voice: Option<&str>) -> windows::core::Result<Vec<u8>> {
    com();
    let synthesizer = SpeechSynthesizer::new()?;
    if let Some(id) = voice {
        for installed in SpeechSynthesizer::AllVoices()? {
            if installed.Id()? == id {
                synthesizer.SetVoice(&installed)?;
                break;
            }
        }
    }
    // Windows 10 1709 and later; older engines read at their own rate.
    if let Ok(options) = synthesizer.Options() {
        let _ = options.SetSpeakingRate(SPEAKING_RATE);
    }
    let stream = wait(synthesizer.SynthesizeTextToStreamAsync(&HSTRING::from(text))?)?;
    let size = u32::try_from(stream.Size()?).unwrap_or(u32::MAX);
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    let loaded = wait(reader.LoadAsync(size)?)?;
    let mut bytes = vec![0_u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

/// Joins this thread to the multithreaded apartment, once.
fn com() {
    thread_local! {
        static JOINED: () = {
            // SAFETY: initializes COM for this thread; a second call (or one
            // in another apartment already) only reports it.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        };
    }
    JOINED.with(|_| {});
}

/// Blocks this (speech worker) thread until `operation` completes;
/// windows-future keeps its blocking join private.
fn wait<T: windows::core::RuntimeType>(
    operation: windows_future::IAsyncOperation<T>,
) -> windows::core::Result<T> {
    while operation.Status()? == windows_future::AsyncStatus::Started {
        std::thread::sleep(Duration::from_millis(2));
    }
    operation.GetResults()
}
