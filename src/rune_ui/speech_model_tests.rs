use super::*;
use rune::text::TextSystem;
use rune::{Env, Runtime};

use crate::app_settings::KeyboardModelChoice;
use crate::transcription::models::{
    self as transcription_models, TranscriptionModelId, TranscriptionSelection,
};

fn runtime(scene: &str) -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    SCENES.iter().find(|(name, _)| *name == scene).unwrap().1(&mut state);
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = crate::rune_ui::offscreen::runtime(
        state,
        view,
        (980.0, 1720.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
        text,
    );
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn tap(rt: &mut Runtime<Ui>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.settle_all();
    rt.frame(0.0);
}

fn settle(rt: &mut Runtime<Ui>) {
    rt.invalidate();
    rt.settle_all();
    rt.frame(0.0);
}

/// The Modes fixture on macOS, with whatever speech switching state the
/// test wants, and Parakeet v3 as the one installed model.
fn modes(macos: bool) -> Runtime<Ui> {
    let mut rt = runtime("modes");
    rt.state.platform.caps = if macos {
        platform::Capabilities::MACOS
    } else {
        platform::Capabilities::LINUX
    };
    rt.state.settings.transcription.language = "en".into();
    // Parakeet v3's last-used language, so a new mode choice starts there.
    rt.state.settings.transcription_recents = vec![TranscriptionSelection {
        model: TranscriptionModelId::ParakeetV3,
        language: "de".into(),
    }];
    rt.state.picker.installed = Some(vec![TranscriptionModelId::ParakeetV3]);
    settle(&mut rt);
    rt
}

fn speech(rt: &Runtime<Ui>, index: Option<usize>) -> Option<KeyboardModelChoice> {
    match index {
        Some(index) => rt.state.settings.dictation_processing.modes[index]
            .speech
            .clone(),
        None => rt
            .state
            .settings
            .dictation_processing
            .default_mode
            .speech
            .clone(),
    }
}

#[test]
fn a_mode_picks_an_installed_speech_model_and_language() {
    let mut rt = modes(true);
    assert!(rt.find("mode-speech-0").is_some());
    assert!(rt.find("mode-speech-language-0").is_none());

    tap(&mut rt, "mode-speech-model-0-trigger");
    // Don't switch, then the installed model.
    tap(&mut rt, "mode-speech-model-0-item1");
    assert_eq!(
        speech(&rt, Some(0)),
        Some(KeyboardModelChoice {
            model: TranscriptionModelId::ParakeetV3,
            language: "de".into(),
        })
    );
    assert!(rt.find("mode-speech-model-0-trigger").is_some());
    assert!(rt.find("mode-speech-language-0-trigger").is_some());

    let french = transcription_models::definition(TranscriptionModelId::ParakeetV3)
        .language_choices()
        .iter()
        .position(|code| *code == "fr")
        .unwrap();
    tap(&mut rt, "mode-speech-language-0-trigger");
    tap(&mut rt, &format!("mode-speech-language-0-item{french}"));
    assert_eq!(speech(&rt, Some(0)).unwrap().language, "fr");

    tap(&mut rt, "mode-speech-model-0-trigger");
    tap(&mut rt, "mode-speech-model-0-item0");
    assert_eq!(speech(&rt, Some(0)), None);
    assert!(rt.find("mode-speech-language-0-trigger").is_none());

    // Global has its own choice.
    tap(&mut rt, "mode-global");
    assert!(rt.find("mode-speech-global").is_some());
    tap(&mut rt, "mode-speech-model-global-trigger");
    tap(&mut rt, "mode-speech-model-global-item1");
    assert_eq!(
        speech(&rt, None),
        Some(KeyboardModelChoice {
            model: TranscriptionModelId::ParakeetV3,
            language: "de".into(),
        })
    );
}

#[test]
fn the_speech_row_needs_a_capable_os() {
    // Linux has no runtime that follows the frontmost app, so saved
    // choices are not offered there.
    let rt = modes(false);
    assert!(rt.find("mode-speech-0").is_none());
    assert!(rt.find("mode-speech-global").is_none());
}
