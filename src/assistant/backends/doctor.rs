//! `hex dev assistant-doctor` (debug builds): prints what the setup sheet
//! would find and runs its self-tests, from this process. Run from a
//! terminal, macOS judges that terminal's permissions, not Hex.app's.

use std::path::PathBuf;

use crate::app_settings::AssistantSettings;
use crate::assistant::{computer, speech};

/// What to do beyond printing what's found.
#[derive(Clone, Debug, Default)]
pub struct Doctor {
    /// Run every self-test for every installed agent (asks each a tiny
    /// prompt, so it uses a little of each account).
    pub checks: bool,
    /// Capture a question's context, writing its screenshot here.
    pub context: Option<PathBuf>,
    /// Read this aloud.
    pub say: Option<String>,
}

pub fn run(doctor: Doctor) -> color_eyre::Result<()> {
    let started = std::time::Instant::now();
    let found = super::detect();
    println!("detect() in {:.1} s:", started.elapsed().as_secs_f32());
    for found in &found {
        println!(
            "  {:<12} ready={:<5} version={:<10} path={} {}",
            super::name(found.backend),
            found.ready,
            found.version.as_deref().unwrap_or("-"),
            found
                .path
                .as_deref()
                .map_or("-".into(), |path| path.display().to_string()),
            found.note.as_deref().unwrap_or_default(),
        );
    }
    println!("agent_desktop(): {:?}", super::agent_desktop());
    println!(
        "computer::permissions_now(): {:?}",
        computer::permissions_now()
    );
    let path = super::agent_env()
        .into_iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, path)| path)
        .unwrap_or_default();
    println!("agent PATH: {path}");
    let voices = speech::voices_with_languages();
    println!("voices: {} installed", voices.len());

    if let Some(file) = &doctor.context {
        let started = std::time::Instant::now();
        let context = computer::context(true);
        println!(
            "context(true) in {} ms: app={:?} window={:?} selected={:?}",
            started.elapsed().as_millis(),
            context.frontmost_app,
            context.window_title,
            context
                .selected_text
                .as_deref()
                .map(|text| super::run::shorten(text, 80)),
        );
        match &context.screenshot {
            Some(shot) => {
                std::fs::write(file, &shot.png)?;
                println!(
                    "  screenshot {}×{} ({} KB) → {}",
                    shot.width,
                    shot.height,
                    shot.png.len() / 1024,
                    file.display()
                );
            }
            None => println!("  no screenshot (see the log)"),
        }
    }

    if doctor.checks {
        let settings = AssistantSettings::default();
        let mut checks = vec![
            super::Check::HexPermissions,
            super::Check::DesktopPermissions,
            super::Check::DesktopRoundTrip,
        ];
        for found in &found {
            let installed = found.path.is_some()
                || found.backend == crate::app_settings::AssistantBackend::ClaudeApi;
            if installed {
                checks.push(super::Check::Answers(found.backend));
                if found.backend != crate::app_settings::AssistantBackend::ClaudeApi {
                    checks.push(super::Check::DesktopThroughAgent(found.backend));
                }
            }
        }
        println!("self-tests:");
        super::self_tests(&checks, &settings, |result| {
            let state = match result.state {
                super::CheckState::Running => {
                    println!("  … {}", result.check.title());
                    return;
                }
                state => state,
            };
            println!(
                "  {:?} {} — {} ({:.1} s){}",
                state,
                result.check.title(),
                result.detail,
                result.took.as_secs_f32(),
                result
                    .fix
                    .map(|fix| format!(" [fix: {}]", fix.label()))
                    .unwrap_or_default(),
            );
        });
    }

    if let Some(text) = &doctor.say {
        let voice = speech::automatic_voice(text);
        println!(
            "speaking {text:?} with {}",
            voice.as_deref().unwrap_or("the system voice")
        );
        speech::speak(text, None);
        while speech::speaking() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    Ok(())
}
