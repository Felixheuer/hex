//! Windows' own voices (`Windows.Media.SpeechSynthesis`, the voices of
//! Settings › Time & language › Speech): each sentence is written to WAV in
//! memory and played by Hex ([`super::writer`]), so queueing, stopping, and
//! the level work as on the Mac.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Media::SpeechSynthesis::SpeechSynthesizer;
use windows::Storage::Streams::DataReader;
use windows::core::HSTRING;

use super::voices::{self, Voice};

/// How fast the voices read (1 is their normal): a little brisker, as on
/// the Mac.
const SPEAKING_RATE: f64 = 1.1;
/// How long a voice list is kept (voices can be added meanwhile).
const VOICES_FRESH: Duration = Duration::from_secs(60);
/// How long listing the voices may take.
const LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// Joins the writer's thread to the multithreaded apartment WinRT needs.
pub fn prepare_thread() {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    // SAFETY: Called once on the writer's own thread, before any WinRT call;
    // an apartment already chosen stays as it is.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

/// Nothing outlives a stop: the sentence being written is cancelled where
/// it waits ([`wait`]).
pub fn stopped() {}

/// The installed voices, listed on a thread of their own (whatever
/// apartment the caller's thread is in, the UI's included).
pub fn voices() -> Vec<Voice> {
    static VOICES: Mutex<Option<(Instant, Vec<Voice>)>> = Mutex::new(None);
    let mut cached = VOICES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((at, voices)) = cached.as_ref()
        && at.elapsed() < VOICES_FRESH
    {
        return voices.clone();
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("hex-speech-voices".into())
        .spawn(move || {
            prepare_thread();
            let _ = sender.send(listed());
        });
    let listed = match spawned {
        Ok(_) => receiver.recv_timeout(LIST_TIMEOUT).ok(),
        Err(_) => None,
    };
    let voices = match listed {
        Some(Ok(listed)) => voices::named(listed),
        Some(Err(error)) => {
            tracing::warn!(%error, "could not list the voices");
            Vec::new()
        }
        None => {
            tracing::warn!("listing the voices took too long");
            Vec::new()
        }
    };
    *cached = Some((Instant::now(), voices.clone()));
    voices
}

fn listed() -> windows::core::Result<Vec<voices::Listed>> {
    let all = SpeechSynthesizer::AllVoices()?;
    (0..all.Size()?)
        .map(|index| {
            let voice = all.GetAt(index)?;
            Ok(super::catalog::windows_listed(
                &voice.Id()?.to_string(),
                &voice.DisplayName()?.to_string(),
                &voice.Language()?.to_string(),
            ))
        })
        .collect()
}

/// `text` read by `voice` (the system's voice for `None`), as samples.
pub fn write(text: &str, voice: Option<&Voice>, generation: u64) -> Option<Vec<f32>> {
    match synthesize(text, voice, generation) {
        Ok(Some(bytes)) => {
            let samples = super::wav::samples(&bytes);
            if samples.is_none() {
                tracing::warn!("the voice wrote audio Hex can't read");
            }
            samples
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "the voice could not read a sentence");
            None
        }
    }
}

/// The WAV `voice` makes of `text`; `None` when a stop came meanwhile.
fn synthesize(
    text: &str,
    voice: Option<&Voice>,
    generation: u64,
) -> windows::core::Result<Option<Vec<u8>>> {
    let synthesizer = SpeechSynthesizer::new()?;
    if let Some(voice) = voice {
        let all = SpeechSynthesizer::AllVoices()?;
        for index in 0..all.Size()? {
            let installed = all.GetAt(index)?;
            if installed.Id()? == voice.id.as_str() {
                synthesizer.SetVoice(&installed)?;
                break;
            }
        }
    }
    // Windows 10 before 1709 has no options: its normal rate reads.
    if let Ok(options) = synthesizer.Options() {
        let _ = options.SetSpeakingRate(SPEAKING_RATE);
    }
    let Some(stream) = wait(
        synthesizer.SynthesizeTextToStreamAsync(&HSTRING::from(text))?,
        generation,
    )?
    else {
        return Ok(None);
    };
    let size = u32::try_from(stream.Size()?).unwrap_or(u32::MAX);
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    let Some(loaded) = wait(reader.LoadAsync(size)?, generation)? else {
        return Ok(None);
    };
    let mut bytes = vec![0; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(Some(bytes))
}

/// Blocks the writer's thread until `operation` completes; a stop cancels
/// it (`None`). windows-future keeps its blocking `join` private, so this
/// polls the status.
fn wait<T: windows::core::RuntimeType>(
    operation: windows_future::IAsyncOperation<T>,
    generation: u64,
) -> windows::core::Result<Option<T>> {
    while operation.Status()? == windows_future::AsyncStatus::Started {
        if super::writer::stopped_since(generation) {
            let _ = operation.Cancel();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    operation.GetResults().map(Some)
}

/// Opens Settings › Time & language › Speech, where voices are added.
pub fn open_voice_settings() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = wide("open");
    let page = wide("ms-settings:speech");
    // SAFETY: Both strings are NUL-terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            page.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as a value above 32.
    if result as isize <= 32 {
        tracing::warn!("could not open the Speech settings");
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
